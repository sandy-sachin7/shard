Output incremental_demo.gif
Require tmux

Set FontSize 14
Set Width 1600
Set Height 800
Set Theme "Catppuccin Mocha"

Hide
Type "tmux kill-session -t side-by-side"
Enter
Type "tmux new-session -s side-by-side -d"
Enter
Type "tmux split-window -h"
Enter
Type "tmux send-keys -t 0 'clear' C-m"
Enter
Type "tmux send-keys -t 1 'clear' C-m"
Enter
Type "tmux attach -t side-by-side"
Enter
Show

Sleep 2s

Type "chmod +x run_git.sh run_shard.sh"
Enter
Type "clear"
Enter

# Run on the left side
Type "./run_git.sh"
Enter

Sleep 1s

# Switch to the right side
Ctrl+B
Type "o"

# Run on the right side
Type "./run_shard.sh"
Enter

# Wait for 22 seconds to capture Git hanging while Shard finishes
Sleep 22s

Hide
Type "tmux kill-server"
Enter
