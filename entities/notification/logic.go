package notification

import (
	"security-camera/db"
	"security-camera/entities/user"

	"go.mongodb.org/mongo-driver/v2/bson"
)

type NotificationLogic struct {
	repo *NotificationRepo
}

func NewNotificationLogic(db *db.DbStruct) *NotificationLogic {
	return &NotificationLogic{
		repo: NewNotificationRepo(db),
	}
}

func (l *NotificationLogic) GetAllNotifications() ([]Notification, error) {
	return l.repo.GetAll()
}

type CreateNotificationRequest struct {
	Message   *string
	User      *user.User
	Timestamp int64

	/**
	* if notification is set ignore User and Message fields
	 */
	Notification *Notification
}

func (l *NotificationLogic) CreateNotification(req CreateNotificationRequest) error {
	if req.Notification != nil {
		return l.repo.Create(*req.Notification)
	}

	userID := bson.NilObjectID
	if req.User != nil {
		userID = req.User.ID
	}
	user := user.User{}
	if req.User != nil {
		user = *req.User
	}

	not := Notification{
		Message: *req.Message,
		UserID:  userID,
		User:    user,
	}
	return l.repo.Create(not)
}

func (l *NotificationLogic) DeleteNotification(id bson.ObjectID) error {
	return l.repo.Delete(id)
}
