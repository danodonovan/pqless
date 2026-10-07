"""Time-to-first-screen and peak memory for Parquet viewers.

Each tool runs in a real pseudo-terminal (160x40). Output is fed through a
terminal emulator (pyte), and the clock stops when a value from the file's
first data row reaches the terminal. Interactive tools are held open for
HOLD seconds, so memory used by background loading counts, then sent their
quit keys. Peak RSS is the tool process's own ru_maxrss from wait4.

Run via `just bench`, which installs the tools and writes the data first:

    python bench.py [TOOL,TOOL...] [FILE,FILE...]
"""

import json
import os
import pty
import select
import signal
import statistics
import struct
import sys
import time
import fcntl
import termios

import pyte

ROWS, COLS = 40, 160
TIMEOUT = 60.0
# Interactive tools stay open this long after their first screen, so
# background loading shows up in peak memory.
HOLD = 5.0
RUNS = 3

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
WORK = f"{REPO}/target/bench"
BIN = f"{WORK}/tools/bin"
VENV = f"{WORK}/venv/bin"
DATA = f"{WORK}/data"
PQLESS = f"{REPO}/target/release/pqless"

# file -> text from the first data row that every tool shows near the left
FILES = {
    "sensors": ("sensors.parquet", "WX-001"),
    "tall": ("tall.parquet", "category-0338"),
    "wide": ("wide.parquet", "7000001"),
}

# name -> (kind, argv template, quit keys)
# Not included: parq 0.1.4 (no data view yet) and parquet-viewer 0.2.0
# (could not open files with timestamp columns).
TOOLS = {
    "pqless": ("pager", [PQLESS, "{f}"], b"q"),
    "visidata": ("pager", [f"{VENV}/vd", "{f}"], b"gq"),
    "tabiew": ("pager", [f"{BIN}/tw", "{f}"], b"q"),
    "parqeye": ("pager", [f"{BIN}/parqeye", "{f}"], b"q"),
    "duckdb": ("head", [f"{BIN}/duckdb", "-c", "SELECT * FROM '{f}' LIMIT 30"], b""),
    "pqrs head": ("head", [f"{BIN}/pqrs", "head", "-n", "30", "{f}"], b""),
    "parquet-cli": ("head", [f"{VENV}/parq", "{f}", "--head", "30"], b""),
    "parquet-tools": ("head", [f"{VENV}/parquet-tools", "show", "--head", "30", "{f}"], b""),
}


# Where a tool displays the first row differently, the text to look for.
# parquet-tools turns the whole wide table into floats: 7000001 -> "7e+06".
TOKEN_OVERRIDES = {("parquet-tools", "wide"): "7e+06"}


def run_once(argv, token, quit_keys, cwd, kind):
    screen = pyte.Screen(COLS, ROWS)
    stream = pyte.ByteStream(screen)
    needle = token.encode()
    start = time.perf_counter()
    pid, fd = pty.fork()
    if pid == 0:
        fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
        os.environ["TERM"] = "xterm-256color"
        os.environ["COLUMNS"], os.environ["LINES"] = str(COLS), str(ROWS)
        os.chdir(cwd)
        try:
            os.execv(argv[0], argv)
        finally:
            os._exit(127)

    # Answer terminal queries (cursor position, device attributes) the way a
    # real terminal would; some tools wait for the reply.
    screen.write_process_input = lambda data: os.write(fd, data.encode())

    ready = None
    exited = None
    tail = b""
    emulate = True  # stop emulating once found, so pyte can't be the bottleneck

    def pump(wait):
        """Reads available output; False once the pty is closed."""
        nonlocal tail, ready
        r, _, _ = select.select([fd], [], [], wait)
        if not r:
            return True
        try:
            data = os.read(fd, 1 << 20)
        except OSError:
            return False
        if not data:
            return False
        if ready is None:
            # The terminal shows bytes as they arrive, so the first time the
            # token is in the stream is when a user would first see it.
            if needle in tail + data:
                ready = time.perf_counter() - start
            tail = (tail + data)[-len(needle):]
        if emulate:
            stream.feed(data)
        return True

    def reap():
        nonlocal exited
        if exited is None:
            done, status, usage = os.wait4(pid, os.WNOHANG)
            if done:
                exited = (status, usage)
        return exited

    alive = True
    deadline = start + TIMEOUT
    while ready is None and alive and time.perf_counter() < deadline:
        alive = pump(0.005)
        if ready is None and token in "\n".join(screen.display):
            ready = time.perf_counter() - start  # drawn with cursor moves
        if ready is None and reap():
            while pump(0.05):  # exited: drain the rest
                pass
            break

    if kind == "pager" and ready is not None:
        hold = time.perf_counter() + HOLD
        while time.perf_counter() < hold and not reap():
            pump(0.02)
        if not reap():
            os.write(fd, quit_keys)
    else:
        emulate = False
    end = max(deadline, time.perf_counter() + 10)
    while not reap() and time.perf_counter() < end:
        pump(0.02)
    if not reap():
        os.kill(pid, signal.SIGKILL)
        exited = os.wait4(pid, 0)[1:]
    os.close(fd)
    status, usage = exited
    screen_text = "\n".join(line.rstrip() for line in screen.display).strip()
    return {
        "seconds": ready,
        "rss_mb": usage.ru_maxrss / 1e6,  # bytes on macOS
        "exit": os.waitstatus_to_exitcode(status),
        "screen": screen_text[:3000],
    }


def main():
    only_tools = sys.argv[1].split(",") if len(sys.argv) > 1 and sys.argv[1] else list(TOOLS)
    only_files = sys.argv[2].split(",") if len(sys.argv) > 2 else list(FILES)
    results = []
    for tool in only_tools:
        kind, template, quit_keys = TOOLS[tool]
        for name in only_files:
            fname, token = FILES[name]
            token = TOKEN_OVERRIDES.get((tool, name), token)
            path = f"{DATA}/{fname}"
            argv = [a.replace("{f}", path) for a in template]
            run_once(argv, token, quit_keys, DATA, kind)  # warm-up
            runs = [run_once(argv, token, quit_keys, DATA, kind) for _ in range(RUNS)]
            ok = [r for r in runs if r["seconds"] is not None]
            row = {
                "tool": tool,
                "kind": kind,
                "file": name,
                "ok": len(ok),
                "seconds": statistics.median(r["seconds"] for r in ok) if ok else None,
                "rss_mb": statistics.median(r["rss_mb"] for r in runs),
                "exit": runs[-1]["exit"],
                "screen": runs[-1]["screen"],
            }
            results.append(row)
            secs = f"{row['seconds']:.3f}s" if row["seconds"] is not None else f">{TIMEOUT:.0f}s / failed"
            print(f"{tool:16} {name:8} {secs:>16} {row['rss_mb']:9.0f} MB  ok={row['ok']}/{RUNS}", flush=True)
    out = f"{WORK}/results-{int(time.time())}.json"
    with open(out, "w") as f:
        json.dump(results, f, indent=1)
    print("saved", out)
    print()
    print(markdown(results, only_files))


def markdown(results, files):
    """Results as a table: one row per tool, one column per file."""
    def cell(r):
        if r["seconds"] is None:
            return "did not finish"
        secs = f"{r['seconds']:.2f} s"
        mem = f"{r['rss_mb'] / 1000:.1f} GB" if r["rss_mb"] >= 1000 else f"{r['rss_mb']:.0f} MB"
        return f"{secs} · {mem}"

    by_key = {(r["tool"], r["file"]): r for r in results}
    tools = list(dict.fromkeys(r["tool"] for r in results))
    lines = ["| Tool | " + " | ".join(files) + " |", "|---" * (len(files) + 1) + "|"]
    for tool in tools:
        cells = [cell(by_key[tool, f]) if (tool, f) in by_key else "" for f in files]
        lines.append(f"| {tool} | " + " | ".join(cells) + " |")
    return "\n".join(lines)


if __name__ == "__main__":
    main()
