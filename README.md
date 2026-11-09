# Security Camera

A small Rust program that turns any Linux machine with a webcam into a motion-detecting security camera controlled from Telegram.

It watches the camera, and when it sees motion above a threshold it sends a short GIF of the motion to your Telegram chat. You can also ask it for a live snapshot, check on the machine, or update it, all by messaging the bot.

**Project page:** https://hypedmike.github.io/security-camera/

## Features

- **Motion detection**, with interchangeable detectors (`DETECTOR` setting or `detector` in `/edit_config`):
  - `background` (default): learns what the empty scene looks like and alarms when a moving blob covers at least `BACKGROUND_THRESHOLD` of the frame. Picks up small or distant movement and ignores lights switching on. The model is in [`lib/detection/`](lib/detection/).
  - `difference`: compares the last frames pixel by pixel and alarms when the average difference reaches `DIFFERENCE_THRESHOLD`.
- **Telegram alerts**: the moment motion is detected a message goes out with the time, the detector, its score and a code. In the background the program records the next 5 s of the camera at 2 frames a second (640x480, dithered) and saves them as a GIF in `gifs/`, at most 1 GB: past that the oldest are deleted (the encoder and the store are in [`lib/gif/`](lib/gif/)). React to the alarm message with 👍 and the GIF is sent as a reply (or send `/hd <code>`). Alarms are only spaced by a 5 s cooldown, there is no cap per minute. In a group the bot must be an administrator to see reactions.
- **Telegram commands**, accepted only from the configured chat:

  | Command   | What it does                                          |
  |-----------|-------------------------------------------------------|
  | `/snap`   | Sends the next camera frame as a photo                |
  | `/info`   | Reports machine state (system) and camera/alarm state |
  | `/alarms` | Lists the latest 10 alarms: whether their GIF is recording, ready or sent (`/queue` still works). The alarm log is in [`lib/alarmlog/`](lib/alarmlog/) |
  | `/hd <code>` | Sends the GIF of the alarm with that code, the same as a 👍 on the alarm message |
  | `/app`    | Sends a button that opens the mini app (live view and motion graph) |
  | `/update` | Pulls the latest code, rebuilds and restarts          |
  | `/pause`  | Stops sending alarms (the camera keeps running)       |
  | `/resume` | Starts sending alarms again                           |
  | `/edit_config [json]` | Without arguments, sends all the settings as JSON (detector, thresholds, frame counts, cooldown, alarm GIF length, speed and quality, snapshot quality). Followed by that JSON, changes them and saves them in `settings.json`, so they survive a restart (`paused` is not saved); keys left out keep their value and nothing changes if any value is invalid |
  | `/reset_config` | Goes back to the default settings (constants and `.env`) and deletes `settings.json` |

- **System alarms**: every minute the program checks the machine and warns on Telegram when the disk is 90% full or more, when the CPU stays at 90% or more for 5 minutes, when a temperature sensor reaches 80°C, and when the connection to Telegram is lost. Each problem is reported once when it starts and once when it is over. A lost connection can only be reported when it comes back ("Connection back after 12m0s (lost at 03:14)"), and a warning that cannot be sent is kept and sent as soon as the connection is back. The limits are in [`lib/securitycamera/src/config.rs`](lib/securitycamera/src/config.rs); values the machine cannot read (e.g. temperature on macOS) are skipped.
- **Daily "i'm alive" message**: every day at 9:00 (machine local time) the bot says it is running, how many alarms there were during the night, from 20:00 the evening before (or that there was no movement), and a suggestion based on today's forecast (for `WEATHER_LATITUDE`/`WEATHER_LONGITUDE`, Lucca if unset) and temperatures from [Open-Meteo](https://open-meteo.com/) (no key needed). On a sunny Sunday it adds a hike suggestion.
- **Live graph** (optional): a web page at `http://localhost:8080` plotting the motion score in real time, handy for tuning the threshold.
- **Telegram mini app** (optional): a page inside Telegram, written in React ([`miniapp-web/`](miniapp-web/)), with the live camera view and the live motion graph. See [Mini app](#mini-app).
- **Guided setup**: missing settings are asked for at startup and saved to `.env`; the camera is chosen interactively and remembered.

## Requirements

- Linux (a V4L2 camera, `/dev/video*`) or macOS (AVFoundation; grant the camera permission when asked). The camera is read by [`lib/my_video/`](lib/my_video/) with [nokhwa](https://crates.io/crates/nokhwa)
- Rust (https://rustup.rs)
- A C toolchain, for the camera library: `sudo apt install build-essential linux-libc-dev`
- A Telegram bot token and your chat id (see below)

## Setup

1. Create a bot with [@BotFather](https://t.me/BotFather) and copy its token.
2. Send a message to your bot, then find your chat id (for example via `https://api.telegram.org/bot<TOKEN>/getUpdates`, look for `chat.id`).
3. Build:

   ```bash
   ./build_linux
   ```

4. Configure (optional, the program asks for anything missing on first run):

   ```bash
   cp .env.example .env
   ```

5. Run:

   ```bash
   ./securitycamera
   ```

On start the bot sends "Security camera is up and running on `<host>`". Press `Ctrl+C` to stop.

## Configuration

Settings live in `.env` (see [`.env.example`](.env.example)).

| Variable                  | Required | Description                                                         |
|---------------------------|----------|---------------------------------------------------------------------|
| `TELEGRAM_BOT_TOKEN`      | yes      | Bot token from @BotFather                                           |
| `TELEGRAM_CHAT_ID`        | yes      | Chat the bot talks to; messages from other chats are ignored        |
| `DETECTOR`                | no       | Motion detector: `background` (default) or `difference`             |
| `BACKGROUND_THRESHOLD`    | no       | Blob area (0 to 1) at or above which the background detector alarms, default `0.003` |
| `DIFFERENCE_THRESHOLD`    | no       | Frame difference (0 to 1) at or above which the difference detector alarms, default `0.05` (the old name `ALARM_THRESHOLD` still works) |
| `CAMERA_ID`               | no       | Camera to use; chosen and saved on first run                        |
| `LIVE_STREAM_GRAPH`       | no       | `1`/`true` serves the live graph on port 8080                       |
| `PRINT_DIFFERENCES`       | no       | Print every frame's motion score to the console                     |
| `PRINT_ALARMS`            | no       | Print a line to the console whenever an alarm is set off            |
| `PRINT_TELEGRAM_MESSAGES` | no       | Print every Telegram message received from the chat                 |
| `MINI_APP_URL`            | no       | Public https address of the mini app; empty turns it off            |

Flag: `-fetch-camera` lists the cameras again and lets you choose a different one.

Fixed settings (cooldown, alarm GIF length and frames a second, size, the GIF storage limit, resolution 640x480, history length) are in [`lib/securitycamera/src/config.rs`](lib/securitycamera/src/config.rs).

## Detectors and thresholds

Each detector has its own score and its own threshold; `/edit_config` changes both (`background_threshold`, `difference_threshold`).

- **background**: frames are shrunk to 320 pixels wide in grayscale and compared with a per-pixel model of the scene (running mean and variance). Pixels that moved away from their usual value are grouped into blobs, and the score is the area of the largest blob as a fraction of the frame: `0.003` is about 230 of the 76,800 detection pixels. A frame where more than half the picture changes at once is taken as a lighting change and ignored while the model relearns, and a score must hold for 3 of the last 5 frames. Anything that changes and then holds still for 60 frames (2–4 s at a webcam's 15–30 fps) becomes part of the background: someone who stops moving, a door left open, or the empty spot left where something stood while the model was learning. So after a big movement the score drops within seconds instead of staying high for a minute and sending an alarm every cooldown.
- **difference**: the average absolute RGB change between consecutive frames over the last `detection_frames` frames (`/edit_config`), normalized from 0 (identical) to 1 (black to white).

To tune one, turn on `LIVE_STREAM_GRAPH`, watch the graph with the room still and with someone moving, and set the threshold between the two.

## Mini app

The mini app is a web page Telegram opens inside the chat: the live view of the camera (MJPEG, about 5 frames a second) and the motion score graph.

1. Make the program reachable over https. Telegram only opens https addresses, and the program only listens on `127.0.0.1:8081`, so nothing is exposed until you do this. With [Tailscale Funnel](https://tailscale.com/kb/1311/tailscale-funnel) (Tailscale installed, HTTPS and Funnel enabled for your tailnet):

   ```bash
   tailscale funnel --bg 8081
   ```

   `tailscale funnel status` shows the public address, like `https://camera.tail1234.ts.net`. Check the exact command and its prerequisites in the Tailscale documentation: it has changed between versions.
2. Put that address in `.env` as `MINI_APP_URL` and restart.
3. Open the **private chat with the bot** (press Start the first time): the menu button next to the message box ("Camera") opens the app. The program sets it at startup for every private chat with the bot, so it works even if `TELEGRAM_CHAT_ID` is a group. `/app` also sends a button for it, but only in a private chat when that is the configured chat: Telegram does not allow these buttons in groups.

A mistake in the mini app settings (an `MINI_APP_URL` that is not https) does not stop the camera: the program starts with the mini app off and says why on Telegram and in the log.

Who gets in: every request carries the signed data Telegram gives the app when it opens (`initData`). The program checks its signature with the bot token, then who the user is:

- if `TELEGRAM_CHAT_ID` is a **private chat**, only that user;
- if it is a **group**, whoever is in the group. Telegram has no call that lists the members of a group, so the program asks it about the user when they open the app (`getChatMember`) and remembers the answer: a yes for 10 minutes, a no for 1 minute. Somebody who joins gets in within a minute, somebody who leaves is out within 10 minutes. If Telegram can't be asked, nobody gets in. Telegram only guarantees this call for other users if **the bot is an administrator of the group**: make it one, or the app will refuse everybody (the log says `can't check whether user ... is in the group`).

The signed data expires after 24 hours. Everybody else, or an expired link, gets "unauthorized". Anyone who knows the public address can still open the page and send requests, so this check is all that protects the video: keep the bot token secret. At most 3 videos are open at once, each stops after 15 minutes (the app pauses it after 10 minutes and while it is hidden), and frames are only encoded while somebody is watching.

The page is built from [`miniapp-web/`](miniapp-web/) (React, TypeScript, Vite) into `lib/securitycamera/dist`, which is committed and embedded in the program, so the machine that runs the camera does not need Node. After changing the page, rebuild it and commit the result:

```bash
cd miniapp-web
pnpm install --frozen-lockfile
pnpm test
pnpm run build
```

The project uses [pnpm](https://pnpm.io/) (the version is the `packageManager` field of `miniapp-web/package.json`, Node 22 or newer). A GitHub Actions workflow ([`.github/workflows/miniapp.yml`](.github/workflows/miniapp.yml)) runs the same steps on every pull request that touches the page: if `lib/securitycamera/dist` is not what the sources build, it commits the rebuilt one to the pull request branch (for a fork, or on `main`, it fails instead).

`pnpm run dev` serves the page with hot reload and sends the API calls to the program on `127.0.0.1:8081`. Outside Telegram the page has no signed data, so it shows "Open this page from Telegram": test it in Telegram, through the tunnel.

## Development

```bash
cd lib
cargo test --release      # the whole workspace, no camera or network needed
cargo clippy --release --all-targets
cargo build --release -p securitycamera
```

The tests of the program talk to a fake Telegram and a fake weather service, run the mini app and live graph servers on local ports, and use a fake camera, so they need neither a webcam nor internet.

Layout: [`lib/securitycamera/`](lib/securitycamera/) is the program, the other crates of the [`lib/`](lib/) workspace are libraries it uses (`alarmlog`, `detection`, `gif`, `my_video`, `recorder`, `streaming`). In `securitycamera/src`:

| Module | What it does |
|--------|--------------|
| `app` | `main`: reads the settings, opens the camera, wires everything together |
| `commands`, `alarms`, `edit_config` | the Telegram commands, what happens on an alarm, the `/edit_config` settings and `settings.json` |
| `telegram` | the Bot API client (messages, uploads, long polling, reactions) |
| `stream`, `clips`, `detector` | the frame loop with its history, the alarm clips, the motion detectors |
| `monitor`, `syswatch`, `sysinfo`, `dailyreport` | the camera counters, the machine alarms and report, the 9:00 message |
| `miniapp`, `livegraph`, `httpd` | the mini app server and its `initData` check, the live graph, the small HTTP server both use |
| `env`, `config`, `update` | `.env` and the first-run questions, the fixed settings, `/update` and the restart |

## Security notes

- The bot only reacts to the configured chat id, but anyone in that chat can run `/update`, which pulls and runs new code. Keep the chat private.
- `.env` contains your bot token and is git-ignored. Never commit it.
