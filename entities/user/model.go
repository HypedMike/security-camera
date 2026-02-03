package user

import "go.mongodb.org/mongo-driver/v2/bson"

type User struct {
	ID         bson.ObjectID `bson:"_id,omitempty"`
	Username   string
	TelegramID string
	ChatID     int64
	Admin      bool
}
