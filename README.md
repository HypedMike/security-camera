# Security Camera

A small Go program that turns any Linux machine with a webcam into a motion-detecting security camera controlled from Telegram.

It watches the camera, and when the picture changes more than a threshold it sends a short GIF of the motion to your Telegram chat. You can also ask it for a live snapshot, check on the machine, or update it, all by messaging the bot.

**Project page:** https://hypedmike.github.io/security-camera/

## Features

- **Motion detection**: compares the last frames pixel by pixel and raises an alarm when the average difference reaches `ALARM_THRESHOLD`.
- **Telegram alerts**: the alarm is sent as an animated GIF with a timestamp and the difference value. Alarms are rate-limited (30 s cooldown).
- **Telegram commands**, accepted only from the configured chat:

  | Command   | What it does                                          |
  |-----------|-------------------------------------------------------|
  | `/snap`   | Sends the next camera frame as a photo                |
  | `/info`   | Reports machine state (system) and camera/alarm state |
  | `/update` | Pulls the latest code, rebuilds and restarts          |

- **Live graph** (optional): a web page at `http://localhost:8080` plotting the frame difference in real time, handy for tuning the threshold.
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
| `ALARM_THRESHOLD`         | yes      | Frame difference (0 to 1) at or above which an alarm is sent, e.g. `0.05` |
| `CAMERA_ID`               | no       | Camera to use; chosen and saved on first run                        |
| `LIVE_STREAM_GRAPH`       | no       | `1`/`true` serves the live graph on port 8080                       |
| `PRINT_DIFFERENCES`       | no       | Print every frame's difference to the console                       |
| `PRINT_TELEGRAM_MESSAGES` | no       | Print every Telegram message received from the chat                 |

Flag: `-fetch-camera` lists the cameras again and lets you choose a different one.

Fixed settings (cooldown, GIF speed, resolution 640x480, history length) are in [`lib/config/config.go`](lib/config/config.go).

## Choosing a threshold

The difference is the average absolute RGB change between consecutive frames, normalized from 0 (identical) to 1 (black to white). Turn on `LIVE_STREAM_GRAPH`, watch the graph with the room still and with someone moving, and set `ALARM_THRESHOLD` between the two.

## Development

```bash
go test ./...
```

Layout: `main.go` wires everything; `telegram_commands.go` defines the bot commands; `lib/` holds the packages (`detection`, `streaming`, `monitor`, `telegram`, `my_video`, `gif`, `livegraph`, `sysinfo`, `update`, `env`, `config`).

## Security notes

- The bot only reacts to the configured chat id, but anyone in that chat can run `/update`, which pulls and runs new code. Keep the chat private.
- `.env` contains your bot token and is git-ignored. Never commit it.
