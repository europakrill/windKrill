/**
 * DOM-grid terminal renderer (M3).
 *
 * Renders SnapshotDto rows as absolutely-positioned text runs inside a
 * monospace grid. M3 scope: correctness + theming; a canvas renderer with
 * ligature/emoji shaping lands in M4.
 */

import type { ScreenEnvelope, SnapshotDto, ColorDto } from "./session";

export interface RendererMetrics {
  charWidth: number;
  lineHeight: number;
}

export class TerminalView {
  readonly root: HTMLElement;
  private readonly screenEl: HTMLElement;
  private readonly cursorEl: HTMLElement;
  private palette: string[] = [];
  private envelope: ScreenEnvelope | null = null;

  constructor(container: HTMLElement) {
    this.root = container;
    this.root.classList.add("kr-terminal");
    this.screenEl = document.createElement("div");
    this.screenEl.className = "kr-screen";
    this.cursorEl = document.createElement("div");
    this.cursorEl.className = "kr-cursor";
    this.root.appendChild(this.screenEl);
    this.root.appendChild(this.cursorEl);
    this.measure();
    window.addEventListener("resize", () => this.measure());
  }

  setPalette(palette: string[]): void {
    this.palette = palette;
  }

  /** Measure the mono font to compute cell size for cursor placement. */
  measure(): void {
    const probe = document.createElement("span");
    probe.className = "kr-measure";
    probe.textContent = "MMMMMMMMMM";
    this.root.appendChild(probe);
    const rect = probe.getBoundingClientRect();
    probe.remove();
    if (rect.width > 0) {
      const charWidth = rect.width / 10;
      this.root.style.setProperty("--kr-char-w", `${charWidth}px`);
      this.root.style.setProperty(
        "--kr-line-h",
        `${Math.round(rect.height)}px`,
      );
    }
  }

  render(envelope: ScreenEnvelope): void {
    this.envelope = envelope;
    this.renderSnapshot(envelope.snapshot);
  }

  get current(): ScreenEnvelope | null {
    return this.envelope;
  }

  private renderSnapshot(snapshot: SnapshotDto): void {
    const frag = document.createDocumentFragment();
    snapshot.lines.forEach((row, rowIndex) => {
      const lineEl = document.createElement("div");
      lineEl.className = "kr-line";
      lineEl.style.top = `calc(var(--kr-line-h) * ${rowIndex})`;
      let col = 0;
      row.runs.forEach((run) => {
        const span = document.createElement("span");
        span.textContent = run.text;
        const attrs = snapshot.attrs[run.attr];
        if (attrs) {
          this.styleSpan(span, attrs);
        } else if (run.attr === 0) {
          // Default attrs need no extra styling.
        }
        span.style.left = `calc(var(--kr-char-w) * ${col})`;
        col += run.text.length;
        lineEl.appendChild(span);
      });
      frag.appendChild(lineEl);
    });
    this.screenEl.replaceChildren(frag);
    this.placeCursor(snapshot);
  }

  private styleSpan(
    span: HTMLElement,
    attrs: { fg: ColorDto; bg: ColorDto; bold: boolean; italic: boolean; underline: boolean; reverse: boolean },
  ): void {
    const fgColor = this.resolve(attrs.fg, "--kr-fg");
    const bgColor = this.resolve(attrs.bg, "");
    if (attrs.reverse) {
      span.style.color = bgColor === "" ? "var(--kr-bg)" : bgColor;
      span.style.backgroundColor = fgColor;
    } else {
      span.style.color = fgColor;
      if (attrs.bg.t !== "default") span.style.backgroundColor = bgColor;
    }
    if (attrs.bold) span.classList.add("kr-bold");
    if (attrs.italic) span.classList.add("kr-italic");
    if (attrs.underline) span.classList.add("kr-underline");
  }

  private resolve(color: ColorDto, fallbackVar: string): string {
    if (color.t === "default") return fallbackVar ? `var(${fallbackVar})` : "";
    if (color.t === "rgb") {
      const [r, g, b] = color.v;
      return `rgb(${r},${g},${b})`;
    }
    return this.palette[color.v] ?? (fallbackVar ? `var(${fallbackVar})` : "");
  }

  private placeCursor(snapshot: SnapshotDto): void {
    if (!snapshot.cursor_visible) {
      this.cursorEl.style.display = "none";
      return;
    }
    this.cursorEl.style.display = "";
    this.cursorEl.style.transform = `translate(calc(var(--kr-char-w) * ${snapshot.cursor_col}), calc(var(--kr-line-h) * ${snapshot.cursor_row}))`;
  }
}
