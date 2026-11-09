// files-plus-menu (herdr-universal-preview): the menu of a file or folder in the
// files-plus panel (right-click on a row, or ⋯ on a preview). Opened as a popup
// with props { path, dir, lang: "en" | "zh" }. ↑/↓ move, Enter runs, Esc closes.
import { View, div, svg } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { props, closeOverlay, theme, exec } from "herdr";

const TEXT = {
  en: { reveal: "Show in Finder", openFolder: "Open in Finder", openFile: "Open with Default App" },
  zh: { reveal: "在访达中显示", openFolder: "在访达中打开", openFile: "用默认应用打开" },
};
const open = (...args) => exec("open", args).catch(() => {});

// [label, icon, action(path)]
function items(p) {
  const tx = TEXT[p?.lang] ?? TEXT.en;
  // A folder opens itself; a file shows selected in its folder, or opens.
  return p?.dir
    ? [[tx.openFolder, "folder", path => open(path)]]
    : [[tx.reveal, "folder", path => open("-R", path)], [tx.openFile, "file-text", path => open(path)]];
}

export default class FilesMenu extends View {
  init(_props, cx) {
    this.at = 0;
    this.handle = cx.focus_handle();
    this.handle.focus();
  }

  choose(i) {
    const p = props(), it = items(p)[i];
    if (!it) return;
    closeOverlay(p?.overlay);
    if (p?.path) it[2](p.path);
  }

  render(cx) {
    const t = theme(), list = items(props());
    return v_flex().id("files-menu").track_focus(this.handle).w_full().min_h(0).p(5).bg(t.popover).text_size(12.8)
      .on_key_down((e, cx) => {
        if (e.key === "down") this.at = (this.at + 1) % list.length;
        else if (e.key === "up") this.at = (this.at + list.length - 1) % list.length;
        else if (e.key === "enter") this.choose(this.at);
        else if (e.key === "escape") closeOverlay(props()?.overlay);
        else return;
        cx.notify();
      })
      .children(list.map((it, i) => h_flex().id(`fm-${i}`).h(31).px(8).gap(9).rounded(7).items_center().text_color(t.foreground)
        .when(i === this.at, el => el.bg(t.selected))
        .hover(s => s.bg(t.hover))
        .on_click(() => this.choose(i))
        .child(svg(`mods/workspace-chrome/${it[1]}.svg`).size(14).flex_none().text_color(t.muted))
        .child(div().flex_1().child(it[0]))));
  }
}
