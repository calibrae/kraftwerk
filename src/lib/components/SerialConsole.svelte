<script>
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";

  let { vmName, onClose } = $props();

  let terminalEl = $state(null);
  let buffer = $state("");
  let connected = $state(false);
  let error = $state(null);
  let unlisten = null;

  onMount(async () => {
    try {
      // Listen for data from the VM
      unlisten = await listen("console:data", (event) => {
        const bytes = new Uint8Array(event.payload);
        const text = new TextDecoder().decode(bytes);
        buffer += text;
        scrollToBottom();
      });

      // Open the console
      await invoke("open_console", { name: vmName });
      connected = true;
    } catch (e) {
      error = e?.message || JSON.stringify(e);
    }
  });

  onDestroy(async () => {
    if (unlisten) unlisten();
    try {
      await invoke("close_console");
    } catch (_) {}
  });

  function scrollToBottom() {
    if (terminalEl) {
      // Use requestAnimationFrame to scroll after render
      requestAnimationFrame(() => {
        terminalEl.scrollTop = terminalEl.scrollHeight;
      });
    }
  }

  async function handleKeydown(e) {
    if (!connected) return;
    e.preventDefault();

    let bytes = null;

    // Map special keys to escape sequences
    if (e.key === "Enter") bytes = [13];
    else if (e.key === "Backspace") bytes = [127];
    else if (e.key === "Tab") bytes = [9];
    else if (e.key === "Escape") bytes = [27];
    else if (e.key === "ArrowUp") bytes = [27, 91, 65];
    else if (e.key === "ArrowDown") bytes = [27, 91, 66];
    else if (e.key === "ArrowRight") bytes = [27, 91, 67];
    else if (e.key === "ArrowLeft") bytes = [27, 91, 68];
    else if (e.key === "Home") bytes = [27, 91, 72];
    else if (e.key === "End") bytes = [27, 91, 70];
    else if (e.key === "Delete") bytes = [27, 91, 51, 126];
    else if (e.key === "PageUp") bytes = [27, 91, 53, 126];
    else if (e.key === "PageDown") bytes = [27, 91, 54, 126];
    else if (e.ctrlKey && e.key.length === 1) {
      // Ctrl+letter → control character
      const code = e.key.toLowerCase().charCodeAt(0) - 96;
      if (code > 0 && code < 27) bytes = [code];
    }
    else if (e.key.length === 1) {
      // Regular character
      bytes = Array.from(new TextEncoder().encode(e.key));
    }

    if (bytes) {
      try {
        await invoke("console_send", { data: bytes });
      } catch (err) {
        error = err?.message || String(err);
      }
    }
  }
</script>

<div class="console-container">
  <div class="console-toolbar">
    <span class="console-title">
      Serial Console — {vmName}
      {#if connected}
        <span class="badge-connected">Connected</span>
      {:else}
        <span class="badge-connecting">Connecting...</span>
      {/if}
    </span>
    <button class="btn-close" onclick={onClose}>Disconnect</button>
  </div>

  {#if error}
    <div class="console-error">{error}</div>
  {/if}

  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <pre
    class="terminal"
    bind:this={terminalEl}
    tabindex="0"
    onkeydown={handleKeydown}
    role="textbox"
    aria-label="Serial console terminal"
  >{buffer}</pre>
</div>

<style>
  .console-container {
    display: flex;
    flex-direction: column;
    height: 100%;
  }

  .console-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 16px;
    background: var(--bg-surface);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }

  .console-title {
    font-size: 13px;
    font-weight: 500;
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .badge-connected {
    display: inline-block;
    padding: 1px 8px;
    border-radius: 10px;
    font-size: 11px;
    background: rgba(52, 211, 153, 0.15);
    color: #34d399;
  }

  .badge-connecting {
    display: inline-block;
    padding: 1px 8px;
    border-radius: 10px;
    font-size: 11px;
    background: rgba(251, 191, 36, 0.15);
    color: #fbbf24;
  }

  .btn-close {
    padding: 4px 12px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-button);
    color: var(--text);
    cursor: pointer;
    font-size: 12px;
    font-family: inherit;
  }

  .btn-close:hover {
    background: #7f1d1d;
    color: #fca5a5;
    border-color: #7f1d1d;
  }

  .console-error {
    padding: 8px 16px;
    background: rgba(239, 68, 68, 0.1);
    border-bottom: 1px solid rgba(239, 68, 68, 0.3);
    color: #ef4444;
    font-size: 12px;
    flex-shrink: 0;
  }

  .terminal {
    flex: 1;
    margin: 0;
    padding: 12px;
    background: #0d0d1a;
    color: #d4d4e8;
    font-family: 'SF Mono', 'Fira Code', 'Cascadia Code', monospace;
    font-size: 13px;
    line-height: 1.4;
    overflow-y: auto;
    overflow-x: hidden;
    white-space: pre-wrap;
    word-break: break-all;
    outline: none;
    cursor: text;
  }

  .terminal:focus {
    box-shadow: inset 0 0 0 1px var(--accent-dim);
  }
</style>
