// files-plus-menu (herdr-universal-preview): the right-click menu of a file or
// folder in the files-plus panel. Opened as a popup with props
// { path, dir, remote }. ↑/↓ move, Enter runs, Esc closes.
import { View, div, svg } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { props, closeOverlay, theme, exec } from "herdr";

// Starts a program and returns at once (Quick Look stays open until closed).
const detached = (...cmd) => exec("sh", ["-c", '"$@" >/dev/null 2>&1 &', "sh", ...cmd]).catch(() => {});
const run = (...cmd) => exec(cmd[0], cmd.slice(1)).catch(() => {});

// [label, icon, action(path, cx)]; null draws a divider.
function items(p) {
  const copy = ["复制路径", "file-text", (path, cx) => cx.write_to_clipboard(path)];
  if (p?.remote) return [copy];
  if (p?.dir) return [
    ["在访达中打开", "folder", path => run("open", path)],
    ["在访达中显示", "search", path => run("open", "-R", path)],
    null,
    copy,
  ];
  return [
    ["放大预览（快速查看）", "square", path => detached("qlmanage", "-p", path)],
    ["用默认应用打开", "panel-right", path => run("open", path)],
    ["在访达中显示", "folder", path => run("open", "-R", path)],
    null,
    copy,
  ];
}

export default class FilesMenu extends View {
  init(_props, cx) {
    this.at = 0;
    this.handle = cx.focus_handle();
    this.handle.focus();
  }

  choose(i, cx) {
    const p = props(), it = items(p).filter(Boolean)[i];
    if (!it) return;
    closeOverlay(p?.overlay);
    if (p?.path) it[2](p.path, cx);
  }

  render(cx) {
    const t = theme(), all = items(props()), count = all.filter(Boolean).length;
    let n = 0;
    return v_flex().id("files-menu").track_focus(this.handle).w_full().min_h(0).p(5).bg(t.popover).text_size(12.8)
      .on_key_down((e, cx) => {
        if (e.key === "down") this.at = (this.at + 1) % count;
        else if (e.key === "up") this.at = (this.at + count - 1) % count;
        else if (e.key === "enter") this.choose(this.at, cx);
        else if (e.key === "escape") closeOverlay(props()?.overlay);
        else return;
        cx.notify();
      })
      .children(all.map((it, k) => {
        if (!it) return div().id(`fm-sep-${k}`).mx(7).my(4).h(1).bg(t.border);
        const i = n++;
        return h_flex().id(`fm-${i}`).h(31).px(8).gap(9).rounded(7).items_center().text_color(t.foreground)
          .when(i === this.at, el => el.bg(t.selected))
          .hover(s => s.bg(t.hover))
          .on_click((_e, cx) => this.choose(i, cx))
          .child(svg(`mods/workspace-chrome/${it[1]}.svg`).size(14).flex_none().text_color(t.muted))
          .child(div().flex_1().child(it[0]));
      }));
  }
}
