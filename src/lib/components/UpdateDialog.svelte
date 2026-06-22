<script>
  /*
   * In-app updater. Driven entirely by tauri-plugin-updater:
   *   check()  → asks the endpoint for latest.json
   *   download_and_install() → pulls the signed bundle, verifies the
   *     minisign signature against the public key baked into the
   *     bundle's tauri.conf.json, swaps the app on disk, then we
   *     restart via tauri-plugin-process.
   *
   * Open flows:
   *   - silent: invoked at launch; closes itself if no update.
   *   - explicit: invoked from the menu; shows "up to date" / errors.
   */
  import { check } from "@tauri-apps/plugin-updater";
  import { relaunch } from "@tauri-apps/plugin-process";

  let { open = $bindable(false), silent = $bindable(false) } = $props();

  let phase = $state("idle"); // idle | checking | available | downloading | done | uptodate | error
  let update = $state(null);
  let progress = $state({ downloaded: 0, total: 0 });
  let err = $state(null);

  async function runCheck() {
    phase = "checking";
    err = null;
    try {
      const u = await check();
      if (!u) {
        if (silent) {
          open = false;
          return;
        }
        phase = "uptodate";
        return;
      }
      update = u;
      phase = "available";
      // If silent and an update is available, surface the dialog.
      // Caller's binding is already true once we got here; flip silent
      // off so the user can interact.
      silent = false;
    } catch (e) {
      if (silent) {
        // Network down, endpoint 404, etc. — don't yell on launch.
        open = false;
        return;
      }
      err = String(e?.message ?? e);
      phase = "error";
    }
  }

  async function install() {
    if (!update) return;
    phase = "downloading";
    progress = { downloaded: 0, total: 0 };
    try {
      await update.downloadAndInstall((event) => {
        if (event.event === "Started") {
          progress = { downloaded: 0, total: event.data.contentLength ?? 0 };
        } else if (event.event === "Progress") {
          progress = {
            downloaded: progress.downloaded + (event.data.chunkLength ?? 0),
            total: progress.total,
          };
        } else if (event.event === "Finished") {
          phase = "done";
        }
      });
      phase = "done";
    } catch (e) {
      err = String(e?.message ?? e);
      phase = "error";
    }
  }

  async function restartNow() {
    try {
      await relaunch();
    } catch (e) {
      err = String(e?.message ?? e);
      phase = "error";
    }
  }

  function close() {
    open = false;
    phase = "idle";
    update = null;
    err = null;
    progress = { downloaded: 0, total: 0 };
  }

  $effect(() => {
    if (open && phase === "idle") {
      runCheck();
    }
  });

  function fmt(bytes) {
    if (!bytes) return "?";
    const mb = bytes / (1024 * 1024);
    return mb >= 1 ? `${mb.toFixed(1)} MB` : `${(bytes / 1024).toFixed(0)} KB`;
  }

  function pct() {
    if (!progress.total) return 0;
    return Math.min(100, Math.round((progress.downloaded / progress.total) * 100));
  }
</script>

{#if open && !silent}
  <div class="overlay" role="dialog" aria-modal="true" aria-label="Software update">
    <div class="dialog">
      <header>
        <h2>Software Update</h2>
        <button class="close" onclick={close} aria-label="Close">×</button>
      </header>

      <div class="body">
        {#if phase === "checking"}
          <p>Checking for updates…</p>
        {:else if phase === "uptodate"}
          <p>Kraftwerk is up to date.</p>
        {:else if phase === "available"}
          <p>
            <strong>Version {update?.version}</strong> is available.
            (current: {update?.currentVersion})
          </p>
          {#if update?.body}
            <pre class="notes">{update.body}</pre>
          {/if}
        {:else if phase === "downloading"}
          <p>Downloading update… {fmt(progress.downloaded)} / {fmt(progress.total)}</p>
          <div class="bar"><div class="bar-fill" style="width: {pct()}%"></div></div>
        {:else if phase === "done"}
          <p>Update installed. Restart to apply.</p>
        {:else if phase === "error"}
          <p class="err">Update failed: {err}</p>
        {/if}
      </div>

      <footer>
        {#if phase === "available"}
          <button class="secondary" onclick={close}>Later</button>
          <button class="primary" onclick={install}>Download &amp; Install</button>
        {:else if phase === "done"}
          <button class="secondary" onclick={close}>Later</button>
          <button class="primary" onclick={restartNow}>Restart Now</button>
        {:else if phase === "uptodate" || phase === "error"}
          <button class="primary" onclick={close}>Close</button>
        {/if}
      </footer>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed; inset: 0;
    background: rgba(0,0,0,0.45);
    display: flex; align-items: center; justify-content: center;
    z-index: 1000;
  }
  .dialog {
    background: var(--bg-panel, #1e1e1e);
    color: var(--fg, #e4e4e4);
    min-width: 420px; max-width: 560px;
    border-radius: 8px;
    box-shadow: 0 10px 40px rgba(0,0,0,0.5);
    display: flex; flex-direction: column;
  }
  header {
    display: flex; align-items: center; justify-content: space-between;
    padding: 1rem 1.25rem; border-bottom: 1px solid rgba(255,255,255,0.08);
  }
  header h2 { margin: 0; font-size: 1.05rem; }
  .close {
    background: none; border: none; color: inherit;
    font-size: 1.4rem; cursor: pointer; line-height: 1;
  }
  .body { padding: 1.25rem; min-height: 4rem; }
  .body p { margin: 0 0 0.75rem; }
  .notes {
    background: rgba(255,255,255,0.05);
    padding: 0.6rem 0.75rem; border-radius: 4px;
    max-height: 12rem; overflow: auto;
    font-size: 0.85rem; white-space: pre-wrap;
  }
  .bar {
    height: 8px; background: rgba(255,255,255,0.08);
    border-radius: 4px; overflow: hidden; margin-top: 0.5rem;
  }
  .bar-fill {
    height: 100%; background: var(--accent, #4a9eff);
    transition: width 0.15s linear;
  }
  .err { color: #ff6b6b; }
  footer {
    display: flex; gap: 0.5rem; justify-content: flex-end;
    padding: 0.75rem 1.25rem; border-top: 1px solid rgba(255,255,255,0.08);
  }
  button.primary, button.secondary {
    padding: 0.4rem 0.9rem; border-radius: 4px;
    border: 1px solid rgba(255,255,255,0.15);
    cursor: pointer; font-size: 0.9rem;
  }
  button.primary {
    background: var(--accent, #4a9eff); color: #fff; border-color: transparent;
  }
  button.secondary { background: transparent; color: inherit; }
</style>
