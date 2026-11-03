# 回退到官方 Herdr

```sh
# 先在 DD-up 中按 ⌘Q（终端和 Agent 继续运行）
scripts/rollback --dry-run     # 查看会删除什么
scripts/rollback               # 卸载 DD-up，然后打开官方 Herdr
```

- 只删除安装清单中、且内容自安装后**没有改动**的项；改动过的项会保留并列出来。
- 默认保留 DD-up 的浏览器资料（登录状态和历史）。要一并删除，执行 `scripts/uninstall --purge-data`。
- 打开官方 Herdr 后，如果它停在已经被删除的 `dd-up` 布局上，按 ctrl-shift-l 切换回 `main`。
- 官方的 `/Applications/Herdr.app`、内置 mod 和布局、工作区始终没有被修改，不需要恢复。
