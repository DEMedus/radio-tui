# radio-tui

Linux TUI for listening to internet radio streams. Stations are listed in a terminal and played with [mpv](https://mpv.io/).

## Run it

Copy the `radio-tui` file onto a **Linux x86_64** machine (USB stick, download, etc.), then:

```bash
chmod +x radio-tui
./radio-tui
```

It plays audio with [mpv](https://mpv.io/). If mpv is missing, radio-tui will say so and offer to install it.

This Linux binary will not run on Windows or macOS.

## Build from source

Needs Rust (via [rustup](https://rustup.rs/)):

```bash
cargo build --release
cp target/release/radio-tui ~/radio-tui
```

## Keys

| Key | Action |
|---|---|
| `j` / `k` or arrows | Move selection |
| Enter | Play selected station |
| Space | Stop / play |
| `+` / `-` | Volume |
| `*` | Favorite the selected station (star, moves to the top). Press again to unfavorite |
| `a` | Add a station (name, then stream URL) |
| `e` | Edit list |
| `i` | Import stations from a remote `stations.json` URL (in edit mode) |
| `u` | Edit the selected station URL (in edit mode) |
| `d` | Delete selected station (in edit mode) |
| `J` / `K` | Move station down / up (in edit mode) |
| `?` | Help |
| `f` | Now-playing screen (also opens after 1 minute idle while playing) |
| `q` / Esc | Quit or leave overlay |
| Ctrl+C | Quit |

After a minute of the same station with no keys, the list is replaced by a now-playing view: station name, ICY track title when the stream sends one, and a full-screen mirrored EQ. Press `f` while playing to open it immediately. Any key returns to the list.

URLs can be pasted into the add prompt. In the URL prompt, Left and Right move the cursor, Backspace deletes the character before it, and Delete removes the character under it.

## Config

Stations and volume are stored at:

```
~/.config/radio-tui/stations.json
```

(`$XDG_CONFIG_HOME/radio-tui/stations.json` when that is set.)

The file is created on first run with a few default streams. Older configs that stored a bare URL list are still loaded and upgraded on the next save.

Press `*` on a highlighted station to mark it with a star and move it to the top. Press `*` again to clear the star; it drops in just after any stations that are still favorited. Favorites stay on this machine — importing a remote list does not copy stars.

You can import extra stations from any static JSON file on the web. In edit mode press `i`, paste a URL like `http://192.168.1.10/stations.json`, pick the ones you want, then **Do it!** (Enter). Selected stations are appended to your local list; nothing is written back to the remote file.

Remote lists use the same shape as the local config:

```json
{
  "stations": [
    { "name": "BBC World Service", "url": "http://stream.live.vc.bbcmedia.co.uk/bbc_world_service" }
  ]
}
```

Try out the sample station list [here](https://raw.githubusercontent.com/DEMedus/radio-tui/main/sample-stations.json).
