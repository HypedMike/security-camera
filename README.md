# Security Camera

A small Go program that turns any Linux machine with a webcam into a motion-detecting security camera controlled from Telegram.

It watches the camera, and when it sees motion above a threshold it sends a short GIF of the motion to your Telegram chat. You can also ask it for a live snapshot, check on the machine, or update it, all by messaging the bot.

**Project page:** https://hypedmike.github.io/security-camera/

## Features

- **Motion detection**, with two interchangeable detectors (`DETECTOR` setting or `detector` in `/edit_config`):
  - `background` (default): learns what the empty scene looks like and alarms when a moving blob covers at least `BACKGROUND_THRESHOLD` of the frame. Picks up small or distant movement and ignores lights switching on.
  - `difference`: compares the last frames pixel by pixel and alarms when the average difference reaches `DIFFERENCE_THRESHOLD`.
- **Telegram alerts**: the alarm is sent as a tiny, low quality animated GIF (about 5 s, 160x120) with a timestamp, the detector, its score and a code. Send `/hd <code>` to get the good quality version (640x480, dithered). The HD GIFs are saved in `gifs/`, at most 1 GB: past that the oldest are deleted and their code stops working. The alarm is sent 3 s after the motion is detected, so the GIF also shows what happens after the first detection. Alarms are rate-limited (5 s cooldown).
- **Telegram commands**, accepted only from the configured chat:

  | Command   | What it does                                          |
  |-----------|-------------------------------------------------------|
  | `/snap`   | Sends the next camera frame as a photo                |
  | `/info`   | Reports machine state (system) and camera/alarm state |
  | `/hd <code>` | Sends the HD version of the alarm GIF with that code |
  | `/update` | Pulls the latest code, rebuilds and restarts          |
  | `/pause`  | Stops sending alarms (the camera keeps running)       |
  | `/resume` | Starts sending alarms again                           |
  | `/edit_config [json]` | Without arguments, sends all the settings as JSON (detector, thresholds, frame counts, cooldown, delay, GIF and snapshot quality). Followed by that JSON, changes them until the next restart; keys left out keep their value and nothing changes if any value is invalid |

- **Live graph** (optional): a web page at `http://localhost:8080` plotting the motion score in real time, handy for tuning the threshold.
- **Guided setup**: missing settings are asked for at startup and saved to `.env`; the camera is chosen interactively and remembered.

## Requirements

- Linux with a V4L2 camera (`/dev/video*`)
- Go 1.24 or newer
- A C toolchain, because the camera driver needs cgo: `sudo apt install build-essential linux-libc-dev`
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
| `PRINT_TELEGRAM_MESSAGES` | no       | Print every Telegram message received from the chat                 |

Flag: `-fetch-camera` lists the cameras again and lets you choose a different one.

Fixed settings (cooldown, GIF speed, size and frame step for both qualities, the GIF storage limit, resolution 640x480, history length) are in [`lib/config/config.go`](lib/config/config.go).

## Detectors and thresholds

Each detector has its own score and its own threshold; `/edit_config` changes both (`background_threshold`, `difference_threshold`).

- **background**: frames are shrunk to 320 pixels wide in grayscale and compared with a per-pixel model of the scene (running mean and variance). Pixels that moved away from their usual value are grouped into blobs, and the score is the area of the largest blob as a fraction of the frame: `0.003` is about 230 of the 76,800 detection pixels. A frame where more than half the picture changes at once is taken as a lighting change and ignored while the model relearns, and a score must hold for 3 of the last 5 frames. Someone who stops moving fades into the background after roughly 900 frames.
- **difference**: the average absolute RGB change between consecutive frames over the last `detection_frames` frames (`/edit_config`), normalized from 0 (identical) to 1 (black to white).

To tune one, turn on `LIVE_STREAM_GRAPH`, watch the graph with the room still and with someone moving, and set the threshold between the two.

## Development

```bash
go test ./...
```

Layout: `main.go` wires everything; `telegram_commands.go` defines the bot commands; `lib/` holds the packages (`detection`, `streaming`, `monitor`, `telegram`, `my_video`, `gif`, `livegraph`, `sysinfo`, `update`, `env`, `config`).

## Security notes

- The bot only reacts to the configured chat id, but anyone in that chat can run `/update`, which pulls and runs new code. Keep the chat private.
- `.env` contains your bot token and is git-ignored. Never commit it.
