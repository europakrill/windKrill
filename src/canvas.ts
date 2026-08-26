/**
 * Canvas terminal renderer (M4).
 *
 * Draws SnapshotDto runs onto a 2D canvas with devicePixelRatio-aware cell
 * metrics. Truecolor attributes (ColorDto Rgb) render losslessly; indexed
 * colors resolve through the active theme palette.
 */

import type { ScreenEnvelope, SnapshotDto } from "./session";

export type ColorDto = {
  t: "default";
  v?: never;
} | {
  t: "indexed";
  v: number;
} | {
  t: "rgb";
  v: [number, number, number];
};

export interface RendererMetrics {
  charWidth: number;
  lineHeight: number;
}

export type Palette = string[];

function colorToCss(
  color: ColorDto | undefined,
  palette: Palette,
  fallback: string,
): string {
  if (!color || color.t === "default") return fallback;
  if (color.t === "rgb") {
    const [r, g, b] = color.v;
    return `rgb(${r},${g},${b})`;
  }
  return palette[color.v] ?? fallback;
}

export class CanvasTerminalView {
  readonly root: HTMLElement;
  private readonly canvas: HTMLCanvasElement;
  private readonly ctx: CanvasRenderingContext2D;
  private palette: Palette = [];
  private envelope: ScreenEnvelope | null = null;
  private charWidth = 9;
  private lineHeight = 18;
  /** Grid size this view's session was created/last-resized with. */
  gridCols = 80;
  gridRows = 24;

  constructor(container: HTMLElement) {
    this.root = container;
    this.root.classList.add("kr-terminal");
    this.canvas = document.createElement("canvas");
    this.canvas.className = "kr-canvas";
    this.root.appendChild(this.canvas);
    const ctx = this.canvas.getContext("2d");
    if (!ctx) throw new Error("canvas 2d context unavailable");
    this.ctx = ctx;
    this.measure();
    window.addEventListener("resize", () => this.scheduleMeasure());
  }

  setPalette(palette: Palette): void {
    this.palette = palette;
    if (this.envelope) this.render(this.envelope);
  }

  /**
   * Compute the grid size that fills the pane and report it when it changes.
   * The caller is responsible for resizing the session (which resizes the
   * PTY); the renderer just needs the target to size its backing store.
   */
  fitToPane(): { cols: number; rows: number } | null {
    const prev = { cols: this.gridCols, rows: this.gridRows };
    const cols = Math.max(8, Math.floor(this.root.clientWidth / this.charWidth));
    const rows = Math.max(2, Math.floor(this.root.clientHeight / this.lineHeight));
    this.gridCols = cols;
    this.gridRows = rows;
    const changed = cols !== prev.cols || rows !== prev.rows;
    // Backing store always tracks the pane so text stays crisp.
    this.measure();
    return changed ? { cols, rows } : null;
  }

  /** Measure the mono font and resize the backing store to fit the pane. */
  measure(): void {
    const probe = document.createElement("span");
    probe.className = "kr-measure";
    probe.textContent = "MMMMMMMMMM";
    this.root.appendChild(probe);
    const rect = probe.getBoundingClientRect();
    probe.remove();
    if (rect.width > 0) {
      this.charWidth = rect.width / 10;
      this.lineHeight = Math.round(rect.height);
    }

    const dpr = window.devicePixelRatio || 1;
    const cssWidth = Math.max(this.root.clientWidth, this.charWidth * 8);
    const cssHeight = Math.max(this.root.clientHeight, this.lineHeight * 2);
    this.canvas.style.width = `${cssWidth}px`;
    this.canvas.style.height = `${cssHeight}px`;
    this.canvas.width = Math.round(cssWidth * dpr);
    this.canvas.height = Math.round(cssHeight * dpr);
    this.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    // Font must be re-set after any canvas resize (state resets).
    this.applyFont();
    if (this.envelope) this.render(this.envelope);
  }

  private scheduleMeasure(): void {
    requestAnimationFrame(() => this.measure());
  }

  private applyFont(): void {
    this.ctx.textBaseline = "middle";
    this.ctx.font = `14px "Cascadia Mono", "Consolas", monospace`;
  }

  render(envelope: ScreenEnvelope): void {
    this.envelope = envelope;
    const snapshot = envelope.snapshot;
    const ctx = this.ctx;

    ctx.clearRect(0, 0, this.canvas.width, this.canvas.height);
    const bg = getComputedStyle(this.root).getPropertyValue("--kr-bg").trim() || "#1e1e1e";
    const fg = getComputedStyle(this.root).getPropertyValue("--kr-fg").trim() || "#cccccc";
    ctx.fillStyle = bg;
    ctx.fillRect(0, 0, this.canvas.width, this.canvas.height);

    snapshot.lines.forEach((row, rowIndex) => {
      let col = 0;
      row.runs.forEach((run) => {
        const attrs = snapshot.attrs[run.attr];
        this.drawRun(run.text, col, rowIndex, attrs, fg, bg);
        col += run.text.length;
      });
    });
    this.drawCursor(snapshot, fg, bg);
  }

  get current(): ScreenEnvelope | null {
    return this.envelope;
  }

  private drawRun(
    text: string,
    col: number,
    row: number,
    attrs:
      | { fg: ColorDto; bg: ColorDto; bold: boolean; italic: boolean; underline: boolean; reverse: boolean }
      | undefined,
    defaultFg: string,
    defaultBg: string,
  ): void {
    const ctx = this.ctx;
    const x = col * this.charWidth;
    const y = row * this.lineHeight + this.lineHeight / 2;

    let fgCss = colorToCss(attrs?.fg, this.palette, defaultFg);
    let bgCss = colorToCss(attrs?.bg, this.palette, defaultBg);
    if (attrs?.reverse) {
      [fgCss, bgCss] = [bgCss || defaultBg, fgCss];
    }
    if (attrs?.bold) fgCss = this.brighten(fgCss);

    if (bgCss) {
      ctx.fillStyle = bgCss;
      ctx.fillRect(x, row * this.lineHeight, text.length * this.charWidth, this.lineHeight);
    }

    ctx.fillStyle = fgCss;
    this.applyFont();
    if (attrs?.italic) ctx.font = `italic ${ctx.font}`;
    if (attrs?.bold) ctx.font = `bold ${ctx.font}`;
    ctx.fillText(text, x, y);
    this.applyFont();

    if (attrs?.underline) {
      ctx.fillRect(x, row * this.lineHeight + this.lineHeight - 3, text.length * this.charWidth, 1);
    }
  }

  /** Bold on the 0-7 range brightens like real terminals; RGB passes through. */
  private brighten(css: string): string {
    const m = /^rgb\((\d+),(\d+),(\d+)$/.exec(css) ?? /^rgb\((\d+),(\d+),(\d+)\)$/.exec(css);
    if (!m) return css;
    const r = Number(m[1]), g = Number(m[2]), b = Number(m[3]);
    if (Math.max(r, g, b) > 64) return css; // already bright-ish
    return `rgb(${Math.min(255, r + 80)},${Math.min(255, g + 80)},${Math.min(255, b + 80)})`;
  }

  private drawCursor(snapshot: SnapshotDto, defaultFg: string, defaultBg: string): void {
    const cursorColor =
      getComputedStyle(this.root).getPropertyValue("--kr-cursor").trim() || "#aeafad";
    const x = snapshot.cursor_col * this.charWidth;
    const y = snapshot.cursor_row * this.lineHeight;
    if (!snapshot.cursor_visible) return;
    const ctx = this.ctx;
    ctx.fillStyle = cursorColor;
    ctx.fillRect(x, y, this.charWidth, this.lineHeight);
    // Invert the glyph under the caret for legibility.
    const ch =
      snapshot.lines[snapshot.cursor_row]?.runs
        .map((r) => r.text)
        .join("")
        .charAt(snapshot.cursor_col) ?? " ";
    ctx.fillStyle = defaultBg || "#1e1e1e";
    ctx.fillText(ch, x, y + this.lineHeight / 2);
  }
}
