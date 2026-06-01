# dmgbuild settings for the Kraftwerk install DMG.
#
# Why dmgbuild and not create-dmg: create-dmg uses an AppleScript bridge
# to ask Finder to lay out the DMG window (icon positions, view options).
# On our CI runner (a LaunchAgent without an active desktop session)
# Finder takes >120s to respond, AppleScript times out, the workflow
# fails (-1712 "AppleEvent timed out"). dmgbuild instead writes the
# .DS_Store directly via biplist, no Finder involvement.
#
# Usage: dmgbuild -s scripts/dmg_settings.py "Kraftwerk" out.dmg
#
# The .app path is picked up from the KRAFTWERK_APP env var so the same
# settings file works for whichever version CI just built.

import os

app_path = os.environ.get("KRAFTWERK_APP", "src-tauri/target/release/bundle/macos/Kraftwerk.app")

# Volume metadata
format = "UDZO"              # compressed read-only
volume_name = "Kraftwerk"

# Layout
icon_size = 128
window_rect = ((200, 120), (600, 380))   # (x, y), (width, height)
text_size = 12
background = "builtin-arrow"             # subtle grey gradient with an arrow

# Files placed at the top level of the DMG
files = [app_path]
symlinks = {"Applications": "/Applications"}

# Icon positions inside the window — coordinates are content-area pixels
# from top-left.
icon_locations = {
    "Kraftwerk.app": (160, 180),
    "Applications": (440, 180),
}
