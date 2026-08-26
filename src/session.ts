/**
 * windKrill session bridge (M3).
 *
 * Thin typed wrapper over the Tauri invoke API. In a plain browser (vite
 * dev without Tauri) `window.__TAURI_INTERNALS__` is absent; the bridge then
 * falls back to a local echo simulator so the UI is developable anywhere.
 */

export interface SessionInfo {
  id: number;
  cols: number;
  rows: number;
  shell: string;
}

export interface AttrDto {
  fg: number;
  bg: number;
  bold: boolean;
  italic: boolean;
  underline: boolean;
  reverse: boolean;
}

export interface RunDto {
  attr: number;
  text: string;
}

export interface RowDto {
  runs: RunDto[];
}

export interface SnapshotDto {
  cols: number;
  rows: number;
  cursor_col: number;
  cursor_row: number;
  cursor_visible: boolean;
  lines: RowDto[];
  attrs: AttrDto[];
}

export interface ScreenEnvelope {
  id: number;
  cols: number;
  rows: number;
  shell: string;
  snapshot: SnapshotDto;
}

export interface StatusDto {
  state: string;
  detail?: string;
}

interface TauriInvoke {
  (cmd: string, args?: Record<string, unknown>): Promise<unknown>;
}

function tauriInvoke(): TauriInvoke | null {
  const internals = (window as unknown as Record<string, unknown>)[
    "__TAURI_INTERNALS__"
  ] as { invoke?: TauriInvoke } | undefined;
  return internals?.invoke ?? null;
}

/** Local echo fallback so the UI runs in a bare browser during development. */
class EchoSimulator {
  private nextId = 1;
  private buffers = new Map<number, string[]>();

  async create(cols: number, rows: number): Promise<SessionInfo> {
    const id = this.nextId++;
    this.buffers.set(id, []);
    return { id, cols, rows, shell: "echo-sim" };
  }

  async input(id: number, data: Uint8Array): Promise<number> {
    const lines = this.buffers.get(id);
    if (lines) {
      const text = new TextDecoder().decode(data);
      for (const ch of text) {
        if (ch === "\r") lines.push("");
        else if (ch >= " ") {
          lines[lines.length - 1] = (lines[lines.length - 1] ?? "") + ch;
        }
      }
    }
    return data.length;
  }

  async screen(id: number, cols: number, rows: number): Promise<ScreenEnvelope> {
    const lines = this.buffers.get(id) ?? [""];
    const visible = lines.slice(-rows);
    while (visible.length < rows) visible.unshift("");
    const snapshot: SnapshotDto = {
      cols,
      rows,
      cursor_col: (visible[visible.length - 1]?.length ?? 0) % cols,
      cursor_row: rows - 1,
      cursor_visible: true,
      lines: visible.map((text) => ({
        runs: [{ attr: 0, text: text.padEnd(cols, " ").slice(0, cols) }],
      })),
      attrs: [
        { fg: -1, bg: -1, bold: false, italic: false, underline: false, reverse: false },
      ],
    };
    return { id, cols, rows, shell: "echo-sim", snapshot };
  }

  async status(id: number): Promise<StatusDto> {
    return this.buffers.has(id)
      ? { state: "running" }
      : { state: "closed" };
  }

  async resize(_id: number, _cols: number, _rows: number): Promise<void> {}
  async close(id: number): Promise<void> {
    this.buffers.delete(id);
  }
}

const echo = new EchoSimulator();

export const bridge = {
  isTauri(): boolean {
    return tauriInvoke() !== null;
  },

  async createSession(cols: number, rows: number): Promise<SessionInfo> {
    const invoke = tauriInvoke();
    if (!invoke) return echo.create(cols, rows);
    return invoke("session_create", { cols, rows }) as Promise<SessionInfo>;
  },

  async sendInput(id: number, data: Uint8Array): Promise<void> {
    const invoke = tauriInvoke();
    if (!invoke) {
      await echo.input(id, data);
      return;
    }
    await invoke("session_input", { id, data: Array.from(data) });
  },

  async screen(id: number): Promise<ScreenEnvelope> {
    const invoke = tauriInvoke();
    if (!invoke) {
      // The simulator needs current size; callers pass it via last known.
      return echo.screen(id, 80, 24);
    }
    return invoke("session_screen", { id }) as Promise<ScreenEnvelope>;
  },

  async status(id: number): Promise<StatusDto> {
    const invoke = tauriInvoke();
    if (!invoke) return echo.status(id);
    return invoke("session_status", { id }) as Promise<StatusDto>;
  },

  async resize(id: number, cols: number, rows: number): Promise<void> {
    const invoke = tauriInvoke();
    if (!invoke) {
      await echo.resize(id, cols, rows);
      return;
    }
    await invoke("session_resize", { id, cols, rows });
  },

  async closeSession(id: number): Promise<void> {
    const invoke = tauriInvoke();
    if (!invoke) {
      await echo.close(id);
      return;
    }
    await invoke("session_close", { id });
  },
};
