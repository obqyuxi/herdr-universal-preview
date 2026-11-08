#!/bin/sh
# Like phone-agent.sh, but a Claude-style menu with a highlight and file paths to tap.
s() { printf '\033]7501;%s\033\\' "$1"; }
s 'state=working:app=phone-agent'
printf '改好了这些文件：\n  README.md\n  src/app.ts:2\n  notes.md\n  logo.png\n'
sleep 1
s 'state=blocked:app=phone-agent:kind=question'
printf '\n下一步做什么？\n\n\033[36m❯ 1. 提交改动\033[0m\n     git commit 并推送\n  2. 再跑一遍测试\n     npm test\n  3. Type something.\n\nEnter to select · Esc to cancel\n'
stty -echo -icanon 2>/dev/null
k=$(dd bs=1 count=1 2>/dev/null)
stty echo icanon 2>/dev/null
printf '\n收到按键：%s\n' "$k"
s 'state=done:app=phone-agent'
sleep 600
