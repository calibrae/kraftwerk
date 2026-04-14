/* tslint:disable */
/* eslint-disable */

/**
 * xterm.js-compatible terminal API.
 */
export class Terminal {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Toggle cursor blink phase. Call from a JS setInterval(~530ms).
     */
    blinkCursor(): void;
    /**
     * Clear selection.
     */
    clearSelection(): void;
    /**
     * Copy selection to clipboard. Call from JS.
     */
    copySelection(): string | undefined;
    /**
     * Dump the grid content as text lines (for debugging).
     */
    dumpGrid(): string;
    fit(): void;
    /**
     * Get the selected text content.
     */
    getSelection(): string | undefined;
    /**
     * Get the URL at pixel position (x, y), if any.
     */
    getUrlAt(x: number, y: number): string | undefined;
    /**
     * Handle a keyboard event. Returns escape sequence or null.
     */
    handleKeyEvent(event: KeyboardEvent): string | undefined;
    /**
     * Handle mousedown — start selection.
     */
    mouseDown(x: number, y: number): void;
    /**
     * Handle mousemove while button held — extend selection.
     */
    mouseMove(x: number, y: number): void;
    /**
     * Handle mouseup — finalize selection.
     */
    mouseUp(): void;
    constructor(options?: object | null);
    /**
     * Register a callback for title changes.
     */
    onTitleChange(callback: Function): void;
    /**
     * Mount the terminal into a DOM container element.
     */
    open(container: HTMLElement): void;
    refresh(): void;
    /**
     * Render if dirty. Call this from requestAnimationFrame.
     * Returns true if a frame was actually drawn.
     */
    render(): boolean;
    reset(): void;
    resize(cols: number, rows: number): void;
    scrollDown(lines: number): void;
    scrollToBottom(): void;
    scrollUp(lines: number): void;
    /**
     * Search grid + scrollback for text. Returns JSON array of matches.
     */
    search(needle: string): string;
    /**
     * Write PTY output data to the terminal. Does NOT render —
     * call `render()` from a rAF callback to batch multiple writes.
     * Returns response bytes to send back to the PTY (DA1, CPR, etc), or null.
     */
    write(data: string): string | undefined;
    /**
     * Write raw bytes to the terminal.
     */
    writeBytes(data: Uint8Array): string | undefined;
    readonly cols: number;
    /**
     * Whether there's an active selection.
     */
    readonly hasSelection: boolean;
    readonly isScrolled: boolean;
    /**
     * Whether the terminal has pending changes to render.
     */
    readonly needsRender: boolean;
    readonly rows: number;
}

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_terminal_free: (a: number, b: number) => void;
    readonly terminal_blinkCursor: (a: number) => void;
    readonly terminal_clearSelection: (a: number) => void;
    readonly terminal_cols: (a: number) => number;
    readonly terminal_copySelection: (a: number, b: number) => void;
    readonly terminal_dumpGrid: (a: number, b: number) => void;
    readonly terminal_fit: (a: number) => void;
    readonly terminal_getUrlAt: (a: number, b: number, c: number, d: number) => void;
    readonly terminal_handleKeyEvent: (a: number, b: number, c: number) => void;
    readonly terminal_hasSelection: (a: number) => number;
    readonly terminal_isScrolled: (a: number) => number;
    readonly terminal_mouseDown: (a: number, b: number, c: number) => void;
    readonly terminal_mouseMove: (a: number, b: number, c: number) => void;
    readonly terminal_mouseUp: (a: number) => void;
    readonly terminal_needsRender: (a: number) => number;
    readonly terminal_new: (a: number) => number;
    readonly terminal_onTitleChange: (a: number, b: number) => void;
    readonly terminal_open: (a: number, b: number) => void;
    readonly terminal_refresh: (a: number) => void;
    readonly terminal_render: (a: number) => number;
    readonly terminal_reset: (a: number) => void;
    readonly terminal_resize: (a: number, b: number, c: number) => void;
    readonly terminal_rows: (a: number) => number;
    readonly terminal_scrollDown: (a: number, b: number) => void;
    readonly terminal_scrollToBottom: (a: number) => void;
    readonly terminal_scrollUp: (a: number, b: number) => void;
    readonly terminal_search: (a: number, b: number, c: number, d: number) => void;
    readonly terminal_write: (a: number, b: number, c: number, d: number) => void;
    readonly terminal_writeBytes: (a: number, b: number, c: number, d: number) => void;
    readonly terminal_getSelection: (a: number, b: number) => void;
    readonly __wbindgen_export: (a: number, b: number) => number;
    readonly __wbindgen_export2: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_export3: (a: number) => void;
    readonly __wbindgen_add_to_stack_pointer: (a: number) => number;
    readonly __wbindgen_export4: (a: number, b: number, c: number) => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
