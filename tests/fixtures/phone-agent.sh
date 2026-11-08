#!/bin/sh
# A pretend agent for phone tests: long mixed-width output, then a question
# (OSC 7501 blocked), echoes the answer, reports done, and repeats.
s() { printf '\033]7501;%s\033\\' "$1"; }
n=0
while :; do
  n=$((n + 1))
  s 'state=working:app=phone-agent'
  printf '\033[1;36m第 %s 轮：\033[0m正在检查 示例项目的标签同步，这一行故意写得很长，用来看手机上的换行和中文宽度是否正确 —— English words mixed in, then a path /Users/me/Documents/github/herdr-universal-preview/mobile/harmony/entry/src/main/ets/pages/Terminal.ets\n' "$n"
  for i in 1 2 3 4 5 6; do printf '  • 第 %s 项：表格 %s 行，已处理 ✓  \033[33mwarn\033[0m %s\n' "$i" $((i * 7)) "$(date +%H:%M:%S)"; done
  s 'state=blocked:app=phone-agent:kind=question:msg=要继续下一轮吗？'
  printf '\n\033[1m要继续下一轮吗？\033[0m 1) 继续  2) 停下  > '
  read -r answer
  printf '收到：%s\n' "$answer"
  s 'state=done:app=phone-agent:msg=第'"$n"'轮完成'
  sleep 2
done
