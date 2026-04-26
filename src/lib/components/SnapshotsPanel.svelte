<script>
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount, onDestroy } from "svelte";

  let { vmName } = $props();

  let snaps = $state([]);
  let loading = $state(false);
  let busy = $state(false);
  let err = $state(null);

  // Create form
  let creating = $state(false);
  let newName = $state("");
  let newDesc = $state("");

  let unlisten = null;

  async function load() {
    if (!vmName) return;
    loading = true;
    err = null;
    try {
      snaps = await invoke("list_snapshots", { name: vmName });
    } catch (e) {
      err = e?.message || String(e);
    } finally {
      loading = false;
    }
  }

  async function createSnap() {
    if (!newName.trim() || busy) return;
    busy = true;
    err = null;
    try {
      await invoke("create_snapshot", {
        name: vmName,
        snapName: newName.trim(),
        description: newDesc.trim() || null,
        flags: 0,
      });
      newName = "";
      newDesc = "";
      creating = false;
      await load();
    } catch (e) {
      err = e?.message || String(e);
    } finally {
      busy = false;
    }
  }

  async function revert(snap) {
    const memNote = snap.has_memory
      ? "\n\nThis snapshot includes RAM, so the VM will resume in the captured state. Network sessions and clock skew may misbehave."
      : "\n\nThis is a disk-only snapshot — the VM will boot fresh after revert.";
    if (!confirm(`Revert ${vmName} to snapshot "${snap.name}"? Current state will be lost.${memNote}`)) return;
    busy = true;
    err = null;
    try {
      // 0 = let libvirt pick post-revert state. Add RUNNING (1) if you
      // want to force resume; PAUSED (2) for paused.
      await invoke("revert_snapshot", { name: vmName, snapName: snap.name, flags: 0 });
      await load();
    } catch (e) {
      err = e?.message || String(e);
    } finally {
      busy = false;
    }
  }

  async function remove(snap, withChildren) {
    const childrenNote = withChildren
      ? `\n\nThis will also delete all ${snap.children?.length || "descendant"} child snapshots.`
      : snap.children?.length
        ? "\n\nChildren will be re-parented to this snapshot's parent."
        : "";
    if (!confirm(`Delete snapshot "${snap.name}"?${childrenNote}`)) return;
    busy = true;
    err = null;
    try {
      await invoke("delete_snapshot", {
        name: vmName,
        snapName: snap.name,
        flags: withChildren ? 1 : 0,
      });
      await load();
    } catch (e) {
      err = e?.message || String(e);
    } finally {
      busy = false;
    }
  }

  // Build a tree from the flat list. Roots have parent_name == null.
  let tree = $derived.by(() => {
    const byName = new Map();
    for (const s of snaps) byName.set(s.name, { ...s, children: [] });
    const roots = [];
    for (const node of byName.values()) {
      if (node.parent_name && byName.has(node.parent_name)) {
        byName.get(node.parent_name).children.push(node);
      } else {
        roots.push(node);
      }
    }
    return roots;
  });

  function formatTime(epoch) {
    if (!epoch) return "—";
    const d = new Date(epoch * 1000);
    return d.toLocaleString();
  }

  onMount(async () => {
    await load();
    // Refresh on lifecycle events too — start/stop changes the disk
    // overlays underneath which may invalidate displayed snapshot state.
    unlisten = await listen("domain_event", (msg) => {
      if (msg?.payload?.vm_name === vmName) load();
    });
  });

  onDestroy(() => {
    if (unlisten) unlisten();
  });

  $effect(() => {
    if (vmName) load();
  });
</script>

<div class="panel">
  <header class="ph">
    <h3>Snapshots <span class="count">{snaps.length}</span></h3>
    <div class="actions">
      <button class="btn-small" onclick={load} disabled={loading || busy}>Refresh</button>
      <button class="btn-primary" onclick={() => { creating = !creating; }} disabled={busy}>
        {creating ? "Cancel" : "+ New Snapshot"}
      </button>
    </div>
  </header>

  {#if err}
    <div class="error">{err}</div>
  {/if}

  {#if creating}
    <div class="create-form">
      <label>
        <span>Name</span>
        <input type="text" bind:value={newName} placeholder="before-upgrade" autofocus />
      </label>
      <label>
        <span>Description (optional)</span>
        <input type="text" bind:value={newDesc} placeholder="why this snapshot" />
      </label>
      <p class="hint">If the VM is running, RAM is captured too. Internal qcow2 snapshots only in v1 — for multi-disk or external snapshots, use virsh.</p>
      <div class="row-actions">
        <button class="btn-small" onclick={() => creating = false} disabled={busy}>Cancel</button>
        <button class="btn-primary" onclick={createSnap} disabled={busy || !newName.trim()}>
          {busy ? "Creating..." : "Create"}
        </button>
      </div>
    </div>
  {/if}

  {#if loading && snaps.length === 0}
    <p class="muted">Loading…</p>
  {:else if snaps.length === 0}
    <p class="muted">No snapshots. Create one above.</p>
  {:else}
    <ul class="tree">
      {#each tree as root (root.name)}
        {@render snapNode(root, 0)}
      {/each}
    </ul>
  {/if}
</div>

{#snippet snapNode(node, depth)}
  <li class="snap" class:current={node.is_current} style="padding-left: {depth * 20}px">
    <div class="snap-row">
      <div class="snap-meta">
        <span class="name mono">{node.name}</span>
        {#if node.is_current}<span class="badge current-badge">CURRENT</span>{/if}
        <span class="state state-{node.state}">{node.state}</span>
        {#if node.has_memory}<span class="badge mem">RAM</span>{/if}
      </div>
      <div class="snap-actions">
        <button class="btn-tiny" onclick={() => revert(node)} disabled={busy}>Revert</button>
        <button class="btn-tiny danger" onclick={() => remove(node, false)} disabled={busy}>Delete</button>
        {#if node.children.length > 0}
          <button class="btn-tiny danger" onclick={() => remove(node, true)} disabled={busy} title="Delete this and all descendants">Delete tree</button>
        {/if}
      </div>
    </div>
    {#if node.description}
      <div class="snap-desc">{node.description}</div>
    {/if}
    <div class="snap-when muted small">{formatTime(node.creation_time)}{node.disk_count ? ` · ${node.disk_count} disk${node.disk_count !== 1 ? "s" : ""}` : ""}</div>
  </li>
  {#each node.children as child (child.name)}
    {@render snapNode(child, depth + 1)}
  {/each}
{/snippet}

<style>
  .panel { padding: 16px; }
  .ph { display: flex; justify-content: space-between; align-items: center; margin-bottom: 12px; }
  .ph h3 { margin: 0; font-size: 14px; font-weight: 600; }
  .count { background: var(--bg-input); color: var(--text); padding: 1px 8px; border-radius: 999px; font-size: 11px; margin-left: 6px; }
  .actions { display: flex; gap: 8px; }

  .create-form {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 12px;
    margin-bottom: 16px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .create-form label { display: flex; flex-direction: column; gap: 4px; font-size: 12px; }
  .create-form span { color: var(--text-muted); }
  .create-form input {
    padding: 6px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-input);
    color: var(--text);
    font-family: inherit;
    font-size: 13px;
  }
  .row-actions { display: flex; justify-content: flex-end; gap: 8px; }

  ul.tree { list-style: none; margin: 0; padding: 0; }
  .snap {
    border-left: 2px solid transparent;
    padding: 8px 12px;
    border-radius: 6px;
  }
  .snap.current {
    border-left-color: #fbbf24;
    background: rgba(251, 191, 36, 0.05);
  }
  .snap-row { display: flex; justify-content: space-between; align-items: center; gap: 12px; }
  .snap-meta { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .name { font-weight: 500; }
  .mono { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }
  .state {
    font-size: 11px;
    padding: 1px 8px;
    border-radius: 999px;
    background: var(--bg-input);
    color: var(--text-muted);
  }
  .state-running { background: rgba(52, 211, 153, 0.15); color: #34d399; }
  .state-paused, .state-pmsuspended { background: rgba(251, 191, 36, 0.15); color: #fbbf24; }
  .state-shutoff { background: rgba(107, 114, 128, 0.20); color: #9ca3af; }
  .state-crashed { background: rgba(239, 68, 68, 0.15); color: #ef4444; }

  .badge {
    font-size: 10px;
    padding: 1px 6px;
    border-radius: 3px;
    background: var(--bg-input);
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .badge.current-badge { background: rgba(251, 191, 36, 0.2); color: #fbbf24; }
  .badge.mem { background: rgba(96, 165, 250, 0.15); color: #60a5fa; }

  .snap-actions { display: flex; gap: 4px; }
  .btn-tiny {
    font-size: 11px;
    padding: 3px 10px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg-button);
    color: var(--text);
    cursor: pointer;
  }
  .btn-tiny:hover:not(:disabled) { background: var(--bg-hover); }
  .btn-tiny:disabled { opacity: 0.5; cursor: not-allowed; }
  .btn-tiny.danger { color: #ef4444; }

  .btn-small, .btn-primary {
    font-size: 12px;
    padding: 5px 12px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--bg-button);
    color: var(--text);
    cursor: pointer;
    font-family: inherit;
  }
  .btn-primary { background: var(--accent); color: white; border-color: var(--accent); }
  .btn-primary:hover:not(:disabled) { filter: brightness(1.1); }
  .btn-primary:disabled, .btn-small:disabled { opacity: 0.5; cursor: not-allowed; }

  .snap-desc { font-size: 12px; color: var(--text); margin-top: 2px; }
  .snap-when { font-size: 11px; color: var(--text-muted); margin-top: 2px; }
  .muted { color: var(--text-muted); }
  .small { font-size: 11px; }

  .hint { font-size: 11px; color: var(--text-muted); margin: 0; }

  .error {
    padding: 8px 12px;
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: 6px;
    color: #ef4444;
    font-size: 12px;
    margin-bottom: 12px;
  }
</style>
