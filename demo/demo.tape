# Records site/demo.gif. Run `just demo`, which builds pqless and the demo
# data first.

Output site/demo.gif

Require pqless

Set Shell "bash"
Set FontSize 15
Set Width 1280
Set Height 660
Set Padding 18
Set Theme "Catppuccin Mocha"
Set TypingSpeed 70ms
Set CursorBlink false

Hide
Type "export PS1='$ ' && cd target/demo && clear" Enter
Sleep 300ms
Show

Sleep 500ms
Type "pqless sensors.parquet"
Sleep 500ms
Enter
Sleep 2.5s

# Page down twice
Type " "
Sleep 1s
Type " "
Sleep 1.5s

# Walk the column cursor right; the status bar names each column
Type@450ms "llllll"
Sleep 1.5s

# Last column, then back to the first
Type "$"
Sleep 2s
Type "0"
Sleep 1.2s

# Last row, then straight to row 125,000
Type "G"
Sleep 2s
Type@120ms "125000g"
Sleep 2.2s

# Help
Type "?"
Sleep 3s
Escape
Sleep 800ms
Type "q"
Sleep 600ms
