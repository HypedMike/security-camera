package notification

import (
	"security-camera/entities/user"

	"go.mongodb.org/mongo-driver/v2/bson"
)

type Notification struct {
	ID bson.ObjectID `bson:"_id,omitempty"`
	// Add other relevant fields for the Notification entity
	Message   string        `bson:"message"`
	Timestamp int64         `bson:"timestamp"`
	Read      bool          `bson:"read"`
	UserID    bson.ObjectID `bson:"user_id"`

	User user.User `bson:"-"`
}
