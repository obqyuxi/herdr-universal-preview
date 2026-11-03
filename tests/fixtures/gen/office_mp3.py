"""Synthetic Office and MP3 samples (no real data). Run through scripts in tests/fixtures/generate.

  uv run --no-project --with python-docx==1.1.2 --with openpyxl==3.1.5 \
         --with python-pptx==1.0.2 --with lameenc==1.8.1 python office_mp3.py OUT_DIR WAV_IN
"""
import math, struct, sys, wave
from pathlib import Path

out = Path(sys.argv[1]); wav_in = Path(sys.argv[2])

# DOCX: 3 pages (explicit page breaks), Chinese text, a table and headings.
from docx import Document
from docx.enum.text import WD_BREAK
d = Document()
for p in range(1, 4):
    d.add_heading(f"第 {p} 页 · Page {p}", level=1)
    d.add_paragraph(f"这是合成的测试文档第 {p} 页。The quick brown fox jumps over the lazy dog.")
    t = d.add_table(rows=3, cols=3)
    t.style = "Table Grid"
    for r in range(3):
        for c in range(3):
            t.cell(r, c).text = f"P{p} R{r + 1}C{c + 1} 单元格"
    for i in range(12):
        d.add_paragraph(f"段落 {i + 1}：用于验证滚动到后续页面的内容。Paragraph {i + 1}.")
    if p < 3:
        d.add_paragraph().add_run().add_break(WD_BREAK.PAGE)
d.save(out / "sample-3pages.docx")

# XLSX: 3 sheets, Chinese headers, numbers, a formula with a cached value absent
# (openpyxl stores formulas without results; previews may show the formula or blank).
from openpyxl import Workbook
wb = Workbook()
for idx, name in enumerate(["销售 Sales", "库存 Stock", "汇总 Summary"]):
    ws = wb.active if idx == 0 else wb.create_sheet()
    ws.title = name
    ws.append(["编号 ID", "名称 Name", "数量 Qty", "单价 Price"])
    for r in range(1, 31):
        ws.append([r, f"{name.split()[0]}项目{r}", r * (idx + 1), round(9.5 + r * 0.25, 2)])
    ws["F1"] = "合计 Total"
    ws["F2"] = "=SUM(C2:C31)"
wb.save(out / "sample-3sheets.xlsx")

# PPTX: 4 slides with titles, Chinese bullets and a shape.
from pptx import Presentation
from pptx.util import Inches, Pt
prs = Presentation()
for s in range(1, 5):
    slide = prs.slides.add_slide(prs.slide_layouts[1])
    slide.shapes.title.text = f"幻灯片 {s} · Slide {s}"
    body = slide.placeholders[1].text_frame
    body.text = f"第 {s} 张幻灯片的要点"
    for b in ("中文要点一", "Second bullet", f"Slide {s} of 4"):
        body.add_paragraph().text = b
    box = slide.shapes.add_textbox(Inches(6), Inches(6), Inches(3), Inches(0.6))
    box.text_frame.text = f"#{s}"
    box.text_frame.paragraphs[0].runs[0].font.size = Pt(28)
prs.save(out / "sample-4slides.pptx")

# MP3: encode the PCM WAV sample with LAME (lameenc).
import lameenc
with wave.open(str(wav_in)) as w:
    ch, rate, frames = w.getnchannels(), w.getframerate(), w.readframes(w.getnframes())
    assert w.getsampwidth() == 2, "expects 16-bit PCM"
enc = lameenc.Encoder()
enc.set_bit_rate(128); enc.set_in_sample_rate(rate); enc.set_channels(ch); enc.set_quality(2)
(out / "sample-speech.mp3").write_bytes(enc.encode(frames) + enc.flush())
print("ok")
