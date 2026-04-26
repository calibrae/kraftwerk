<script>
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";

  let { vmName } = $props();

  let originalXml = $state("");
  let editXml = $state("");
  let loading = $state(false);
  let saving = $state(false);
  let err = $state(null);
  let savedNote = $state(null);

  let modified = $derived(editXml !== originalXml);

  async function load() {
    if (!vmName) return;
    loading = true;
    err = null;
    savedNote = null;
    try {
      // Always edit the inactive (persistent) definition. The live XML
      // contains runtime-only values libvirt fills in (graphics port=
      // assigned, source paths normalised, etc.) which would round-trip
      // poorly.
      const xml = await invoke("get_domain_xml", { name: vmName, inactive: true });
      originalXml = xml;
      editXml = xml;
    } catch (e) {
      err = e?.message || String(e);
    } finally {
      loading = false;
    }
  }

  async function save() {
    if (!modified || saving) return;
    if (!confirm(
      "Replace the persistent definition of " + vmName + " with the edited XML?\n\n" +
      "The running VM is unaffected until next start. If the XML is invalid, libvirt will reject it and the existing definition stays intact."
    )) return;
    saving = true;
    err = null;
    savedNote = null;
    try {
      await invoke("define_domain", { xml: editXml });
      // Re-fetch to absorb any normalisation libvirt did.
      const fresh = await invoke("get_domain_xml", { name: vmName, inactive: true });
      originalXml = fresh;
      editXml = fresh;
      savedNote = "Saved. libvirt may have normalised whitespace or attribute order — re-rendered below.";
    } catch (e) {
      err = e?.message || String(e);
    } finally {
      saving = false;
    }
  }

  function reset() {
    if (modified && !confirm("Discard your edits?")) return;
    editXml = originalXml;
    err = null;
    savedNote = null;
  }

  // Tab key inserts two spaces instead of focus-shifting. Standard
  // textarea quality-of-life for editing code in plain HTML.
  function onKeyDown(e) {
    if (e.key === "Tab") {
      e.preventDefault();
      const el = e.target;
      const start = el.selectionStart;
      const end = el.selectionEnd;
      editXml = editXml.slice(0, start) + "  " + editXml.slice(end);
      // restore caret after the inserted spaces
      requestAnimationFrame(() => {
        el.selectionStart = el.selectionEnd = start + 2;
      });
    }
  }

  onMount(load);

  $effect(() => {
    if (vmName) load();
  });
</script>

<div class="panel">
  <header class="ph">
    <h3>
      Raw XML
      {#if modified}<span class="badge mod">MODIFIED</span>{/if}
    </h3>
    <div class="actions">
      <button class="btn-small" onclick={load} disabled={loading || saving}>
        {loading ? "Loading..." : "Refresh"}
      </button>
      <button class="btn-small" onclick={reset} disabled={!modified || saving}>Reset</button>
      <button class="btn-primary" onclick={save} disabled={!modified || saving || loading}>
        {saving ? "Saving..." : "Save"}
      </button>
    </div>
  </header>

  {#if err}
    <pre class="error">{err}</pre>
  {/if}
  {#if savedNote}
    <div class="ok">{savedNote}</div>
  {/if}

  <p class="hint">
    Editing the persistent definition only; running VM unaffected until next start.
    <code>VIR_DOMAIN_XML_INACTIVE</code> view, what kraftwerk's panels work against.
    Use this for anything we don't model yet (advanced CPU pinning, custom devices, mdev, etc.). Save will fail loudly if the XML is invalid.
  </p>

  <textarea
    class="xml-edit mono"
    bind:value={editXml}
    onkeydown={onKeyDown}
    spellcheck="false"
    autocorrect="off"
    autocapitalize="off"
    placeholder={loading ? "Loading…" : ""}></textarea>
</div>

<style>
  .panel { padding: 16px; display: flex; flex-direction: column; height: 100%; box-sizing: border-box; }
  .ph { display: flex; justify-content: space-between; align-items: center; margin-bottom: 12px; }
  .ph h3 { margin: 0; font-size: 14px; font-weight: 600; display: flex; align-items: center; gap: 8px; }
  .actions { display: flex; gap: 8px; }
  .badge.mod { background: rgba(251, 191, 36, 0.20); color: #fbbf24; padding: 1px 8px; border-radius: 4px; font-size: 10px; letter-spacing: 0.05em; }

  .btn-small, .btn-primary {
    font-size: 12px; padding: 5px 12px; border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--bg-button); color: var(--text);
    cursor: pointer; font-family: inherit;
  }
  .btn-small:hover:not(:disabled) { background: var(--bg-hover); }
  .btn-primary { background: var(--accent); color: white; border-color: var(--accent); }
  .btn-primary:hover:not(:disabled) { filter: brightness(1.1); }
  .btn-small:disabled, .btn-primary:disabled { opacity: 0.5; cursor: not-allowed; }

  .hint { color: var(--text-muted); font-size: 11px; margin: 0 0 8px; }
  .hint code { font-size: 11px; background: var(--bg-input); padding: 1px 4px; border-radius: 3px; }

  .xml-edit {
    flex: 1;
    min-height: 400px;
    width: 100%;
    box-sizing: border-box;
    background: var(--bg-input);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 12px;
    font-size: 12px;
    line-height: 1.45;
    resize: vertical;
    tab-size: 2;
    -moz-tab-size: 2;
  }
  .xml-edit:focus {
    outline: none;
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent-dim);
  }
  .mono { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }

  .error {
    margin: 0 0 8px;
    padding: 8px 12px;
    background: rgba(239, 68, 68, 0.08);
    border: 1px solid rgba(239, 68, 68, 0.30);
    border-radius: 6px;
    color: #ef4444;
    font-size: 11px;
    white-space: pre-wrap;
    max-height: 160px;
    overflow: auto;
  }
  .ok {
    margin: 0 0 8px;
    padding: 6px 10px;
    background: rgba(52, 211, 153, 0.08);
    border: 1px solid rgba(52, 211, 153, 0.30);
    border-radius: 6px;
    color: #34d399;
    font-size: 12px;
  }
</style>
