#!/bin/sh
# Reports OSC 7501 statuses like an agent: working, blocked (permission), working, done.
s() { printf '\033]7501;%s\033\\' "$1"; sleep "${2:-1}"; }
sleep 1
s 'state=working:app=fake-agent'
s 'state=blocked:app=fake-agent:kind=permission'
s 'state=working:app=fake-agent'
s 'state=done:app=fake-agent' 30
