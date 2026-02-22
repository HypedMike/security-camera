![mountain](https://images.pexels.com/photos/15942308/pexels-photo-15942308.jpeg)

# Security Camera with Motion Detection & Telegram Alerts

A robust Go-based security camera system that captures video from a webcam, detects motion using computer vision, and sends instant alerts with snapshots via Telegram to admin users.

## Features

- 🎥 Real-time webcam video capture using Linux V4L2 (no OpenCV required)
- 🔍 Intelligent motion detection with frame similarity analysis
- 📱 Telegram bot integration for instant alerts with images
- 💾 MongoDB database for user and notification management
- 👥 User registration system via Telegram
- 🔐 Admin-only alert notifications
- ⚡ Headless server mode for production environments
- 🎯 Configurable sensitivity and detection parameters
- 🚫 Built-in notification logic to prevent spam

## Architecture

```
security-camera/
├── cmd/
│   └── server/        # Main application entry point
├── camera/            # Camera service and motion detection
│   ├── main.go        # Camera service initialization
│   ├── webcam.go      # Webcam capture and frame processing
│   └── trigger.go     # Trigger system (Telegram alerts)
├── db/                # Database connection management
├── telegram/          # Telegram bot integration
├── entities/
│   ├── notification/  # Notification business logic
│   └── user/          # User model and repository
└── go.mod
```

## Technology Stack

- **Go 1.21+** - Core application
- **blackjack/webcam** - Linux V4L2 webcam capture (no OpenCV dependency)
- **MongoDB** - Database for users and notifications
- **Telegram Bot API** - Real-time alert system
- **godotenv** - Environment configuration

## Prerequisites

### System Requirements

- Go 1.21 or higher
- Linux with V4L2-compatible webcam (`/dev/video0`)
- MongoDB instance (local or cloud)
- Telegram Bot Token

> **Note:** OpenCV is no longer required. The application uses the Linux V4L2 kernel
> interface directly via `github.com/blackjack/webcam` for webcam capture, and
> standard Go libraries for frame processing (MJPEG and YUYV formats supported).

### Install Go Dependencies

```bash
go mod download
```

## Configuration

### Environment Variables

Create a `.env` file in the project root with the following variables:

```env
# Telegram Bot Configuration
TELEGRAM_API_KEY=your_telegram_bot_token_here

# MongoDB Configuration
MONGODB_URI=mongodb://localhost:27017
DB_NAME=security_camera
```

### Getting a Telegram Bot Token

1. Open Telegram and search for [@BotFather](https://t.me/botfather)
2. Send `/newbot` and follow the instructions
3. Copy the API token provided
4. Add the token to your `.env` file

### MongoDB Setup

**Local MongoDB:**

```bash
# Install MongoDB (macOS)
brew install mongodb-community
brew services start mongodb-community

# Install MongoDB (Ubuntu)
sudo apt-get install mongodb
sudo systemctl start mongodb
```

**MongoDB Atlas (Cloud):**

1. Create a free account at [MongoDB Atlas](https://www.mongodb.com/cloud/atlas)
2. Create a cluster and get the connection string
3. Update `MONGODB_URI` in `.env` with your connection string

## Usage

### Running the Application

```bash
# Run the server
go run cmd/server/main.go
```

The application will:

1. Connect to MongoDB
2. Initialize the Telegram bot
3. Start the camera service with motion detection
4. Begin monitoring for motion in headless mode

### Debug Mode with VS Code

The project includes a debug configuration in `.vscode/launch.json` that automatically loads environment variables:

1. Open the project in VS Code
2. Press `F5` or go to Run > Start Debugging
3. The debugger will load `.env` variables automatically

### User Registration

To receive motion alerts:

1. Start the application
2. Open Telegram and find your bot
3. Send any message to the bot (e.g., `/start` or `hello`)
4. The bot will register you in the database
5. To receive alerts, an admin must set your `admin` field to `true` in MongoDB

### Setting Admin Users

Connect to MongoDB and update a user:

```javascript
// MongoDB shell
use security_camera

// Set a user as admin by TelegramID
db.users.updateOne(
  { "telegramid": "YOUR_TELEGRAM_ID" },
  { $set: { "admin": true } }
)

// Or by username
db.users.updateOne(
  { "username": "your_username" },
  { $set: { "admin": true } }
)
```

## How It Works

### Motion Detection Algorithm

1. **Frame Capture**: Continuously captures frames from the webcam (device ID 0)
2. **Buffer Management**: Maintains a rolling buffer of the last 50 frames
3. **Similarity Analysis**: Every 100 frames, calculates frame-to-frame similarity
4. **Motion Threshold**: If similarity drops below 99% (indicating significant changes), motion is detected
5. **Alert Trigger**: Captures the frame as a JPEG and sends it via Telegram to all admin users

### Frame Similarity Calculation

The system uses pure Go pixel difference analysis (no external library needed):

```
similarity = 1.0 - (actual_pixel_difference / max_possible_difference)
```

- `1.0` = Identical frames (no motion)
- `< 0.99` = Significant difference (motion detected)

### Components

#### Camera Service

- **WebcamService**: Manages webcam capture and frame processing
- **CameraService**: Orchestrates the camera and trigger system
- **IWebcamTrigger**: Interface for motion detection callbacks

#### Telegram Integration

- **TelegramBot**: Bot initialization and message handling
- **TelegramTrigger**: Implements motion detection trigger to send alerts
- Automatic user registration on first interaction
- Image attachment support for motion snapshots

#### Database Layer

- **DbStruct**: MongoDB connection wrapper
- **UserRepository**: CRUD operations for users
- **NotificationRepo**: Notification persistence (optional logging)

## Project Structure Details

### `/cmd/server/main.go`

Application entry point that:

- Loads environment variables
- Initializes database connection
- Starts Telegram bot
- Creates camera service with Telegram trigger
- Begins motion detection loop

### `/camera/`

- **main.go**: Camera service initialization and orchestration
- **webcam.go**: Core motion detection logic, frame capture, similarity calculation
- **trigger.go**: Telegram alert implementation

### `/telegram/main.go`

- Bot initialization and lifecycle management
- User registration handler
- Alert sending to admin users with images

### `/db/main.go`

- MongoDB client setup
- Connection management
- Collection and context helpers

### `/entities/`

- **user/**: User model and database operations
- **notification/**: Notification business logic and persistence

## API Reference

### CameraService

```go
// Create a new camera service
func NewCameraService(db *db.DbStruct, trigger IWebcamTrigger) *CameraService

// Start the webcam monitoring service
func (cs *CameraService) StartWebcamService() error

// Close and cleanup resources
func (cs *CameraService) Close()
```

### TelegramBot

```go
// Create a new Telegram bot instance
func NewTelegramBot(db *db.DbStruct, apiKey string) (*TelegramBot, error)

// Send alert to admin users
func (tb *TelegramBot) SendAlert(text string, options SendMessageOptions) error

// Stop the bot gracefully
func (tb *TelegramBot) Stop()
```

### WebcamService

```go
// Create a new webcam service
func NewWebcamService(showDisplay bool, db *db.DbStruct) (*WebcamService, error)

// Set the trigger for motion detection callbacks
func (ws *WebcamService) SetTrigger(trigger IWebcamTrigger)

// Start processing frames for motion detection
func (ws *WebcamService) ElaborateFrames(maxFrames int)

// Listen and capture frames from webcam
func (ws *WebcamService) ListenToFrames() error
```

## Configuration Options

### Display Mode

The camera service can run in two modes:

- **Headless Mode** (default): `showDisplay = false` - No GUI, suitable for servers
- **Display Mode**: `showDisplay = true` - Shows live camera feed window

⚠️ **Note**: Display mode must run on the main thread (macOS requirement)

### Detection Sensitivity

Adjust sensitivity in [camera/webcam.go](camera/webcam.go#L145):

```go
// Current threshold: similarity < 0.99 triggers motion
if similarity*10-9 < 0.99 && ws.trigger != nil {
    (*ws.trigger).OnMovementDetected(img)
}
```

Lower values = more sensitive (detects smaller movements)
Higher values = less sensitive (only detects major changes)

### Frame Processing

Configure in [camera/main.go](camera/main.go):

```go
go webcamService.ElaborateFrames(100)  // Process every 100 frames
```

Lower value = faster detection but more CPU usage
Higher value = slower detection but less CPU usage

## Database Schema

### Users Collection

```javascript
{
  "_id": ObjectId,
  "username": String,
  "telegramid": String,  // Telegram user ID
  "chatid": Number,      // Telegram chat ID
  "admin": Boolean       // Admin flag for alerts
}
```

### Notifications Collection (Optional)

```javascript
{
  "_id": ObjectId,
  "message": String,
  "userid": ObjectId,
  "timestamp": Number,
  "user": Object
}
```

## Troubleshooting

### Camera Not Opening

```bash
# Check camera permissions (macOS)
# Go to System Preferences > Security & Privacy > Camera

# Test camera device
ls -la /dev/video*

```

### MongoDB Connection Issues

```bash
# Test MongoDB connection
mongosh "your_connection_string"

# Check if MongoDB is running (local)
brew services list | grep mongodb
sudo systemctl status mongodb
```

### Telegram Bot Not Responding

- Verify `TELEGRAM_API_KEY` is correct in `.env`
- Check bot token with [@BotFather](https://t.me/botfather) using `/token`
- Ensure bot is not stopped or revoked
- Check application logs for connection errors

### Motion Detection Too Sensitive

Adjust the threshold in [camera/webcam.go](camera/webcam.go#L145):

```go
// Increase 0.99 to 0.995 for less sensitivity
if similarity*10-9 < 0.995 && ws.trigger != nil {
```

### OpenCV/GoCV Errors

OpenCV and GoCV are no longer used. If you encounter webcam access issues, ensure your user has read/write access to the V4L2 device:

```bash
# Check camera device
ls -la /dev/video*

# Add your user to the video group (if needed)
sudo usermod -aG video $USER
```

## Performance Considerations

- **Frame Buffer**: Default 50 frames (~2 seconds at 30fps)
- **Processing Interval**: Every 100 frames (~3 seconds)
- **Memory Usage**: ~10-50MB depending on resolution
- **CPU Usage**: 5-15% on modern processors

## Security Recommendations

1. **Secure Environment Variables**: Never commit `.env` files
2. **MongoDB Authentication**: Use authentication in production
3. **Network Security**: Run on private network or VPN
4. **Bot Token**: Keep Telegram bot token secret
5. **Admin Access**: Carefully control who has admin privileges

## Future Enhancements

- [ ] Multiple camera support
- [ ] Recording functionality with video storage
- [ ] Web dashboard for viewing alerts
- [ ] Custom detection zones/masking
- [ ] Alert scheduling (quiet hours)
- [ ] Mobile app integration
- [ ] Cloud storage for images
- [ ] AI-powered object detection (person vs pet)

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## License

MIT

## Acknowledgments

- [GoCV](https://gocv.io/) - Original Go bindings for OpenCV (replaced)
- [go-telegram-bot](https://github.com/go-telegram/bot) - Telegram Bot API
- [MongoDB Go Driver](https://github.com/mongodb/mongo-go-driver) - Official MongoDB driver

## Support

For issues and questions:

- Open an issue on GitHub
- Check existing documentation and issues first
- Provide logs and configuration details when reporting bugs

---

Built with ❤️ using Go and Linux V4L2
