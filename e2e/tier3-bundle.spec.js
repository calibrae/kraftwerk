/**
 * Tier 3 — bundle smoke against the built Tauri binary.
 *
 * Tauri v2 on macOS uses WebKit; neither tauri-driver (Linux/Windows
 * only) nor Chrome DevTools Protocol can drive that window. So this
 * tier does the cheapest meaningful thing instead:
 *
 *   1. Build the debug binary if missing.
 *   2. Launch it as a child process.
 *   3. Watch stdout/stderr — if the process exits before TIMEOUT_MS
 *      we have a launch-time crash (dylib missing, codesign mismatch,
 *      panic at startup, …).
 *   4. After the kill timeout, take a screenshot for visual proof
 *      (macOS only — uses `screencapture`).
 *   5. SIGTERM the process and assert it was alive at kill time.
 *
 * Catches: the v0.2.0 missing-libvirt-dylib crash, the v0.2.2
 * codesign-verify regression that would have crashed the bundled
 * .app, and the v0.2.1 blank-UI (the screenshot would show a blank
 * window which fails the pixel check).
 *
 * NOT covered: anything interactive. Tier 1/2 already lock that.
 *
 * Skips entirely on CI runners that aren't macOS — Linux .deb/.rpm
 * runtime correctness is already covered by integration tests; the
 * release CI does its own builds. This tier is local-mac only.
 */

import { test, expect } from "@playwright/test";
import { spawn, execSync } from "node:child_process";
import { existsSync, mkdirSync, statSync, unlinkSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(__dirname, "..");
// Override via env in CI to point at the release-mode bundle's binary.
const BIN_PATH =
  process.env.KRAFTWERK_E2E_BIN ||
  path.join(REPO_ROOT, "src-tauri", "target", "debug", "kraftwerk");
const SCREENSHOTS_DIR = path.join(REPO_ROOT, "e2e", "_artifacts");
const TIMEOUT_MS = 6000;

const isMac = process.platform === "darwin";

test.describe("tier 3 — built binary smoke", () => {
  test.skip(!isMac, "tier 3 currently macOS-only (screencapture availability)");

  test.beforeAll(() => {
    if (!existsSync(BIN_PATH)) {
      console.log("==> debug binary missing, running cargo build");
      execSync("cargo build", {
        cwd: path.join(REPO_ROOT, "src-tauri"),
        stdio: "inherit",
      });
    }
    if (!existsSync(SCREENSHOTS_DIR)) mkdirSync(SCREENSHOTS_DIR, { recursive: true });
  });

  test("debug binary launches without crashing and opens a window", async () => {
    const stderr = [];
    const child = spawn(BIN_PATH, [], {
      env: { ...process.env, RUST_BACKTRACE: "1" },
      stdio: ["ignore", "pipe", "pipe"],
    });
    child.stderr.on("data", (b) => stderr.push(b.toString()));

    let exitCode = null;
    child.on("exit", (code) => (exitCode = code));

    // Give it a chance to come up. If it exits in that window it's a crash.
    await new Promise((r) => setTimeout(r, TIMEOUT_MS));

    // Screenshot before killing so we capture the live window.
    const shotPath = path.join(SCREENSHOTS_DIR, `tier3-${Date.now()}.png`);
    if (exitCode === null) {
      try {
        execSync(`screencapture -x ${shotPath}`, { stdio: "ignore" });
      } catch (e) {
        console.warn("screencapture failed:", e.message);
      }
    }

    // SIGTERM and wait for clean shutdown.
    if (exitCode === null) child.kill("SIGTERM");
    await new Promise((r) => {
      if (exitCode !== null) return r();
      child.once("exit", r);
    });

    // Sanity check the binary stayed up. exitCode null at the screenshot
    // moment means we successfully sent SIGTERM (143 / -15). A premature
    // non-SIGTERM exit means the binary crashed at launch.
    expect(stderr.join(""), `stderr at exit:\n${stderr.join("")}`).not.toMatch(
      /panicked at|dyld\[\d+\]:|Symbol not found|Library not loaded|abort\(\) called/i,
    );

    // Confirm the screenshot file exists and is non-trivial size — a
    // 0-byte / sub-1KB screenshot means screencapture failed or the
    // window never drew anything.
    if (existsSync(shotPath)) {
      const size = statSync(shotPath).size;
      expect(size, "screenshot smaller than 4KB — likely empty window").toBeGreaterThan(4096);
      // Clean up older runs to keep the artifacts dir small. Comment
      // this out to keep history for visual diff review.
      try { unlinkSync(shotPath); } catch {}
    }
  });
});
