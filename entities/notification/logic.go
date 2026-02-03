package notification

import (
	"errors"
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
	Message *string
	User    *user.User

	/**
	* if notification is set ignore User and Message fields
	 */
	Notification *Notification
}

func (l *NotificationLogic) CreateNotification(req CreateNotificationRequest) error {
	if req.Notification != nil {
		return l.repo.Create(*req.Notification)
	}
	if req.User == nil || req.Message == nil {
		return errors.New("user and message must be provided if notification is not set")
	}
	not := Notification{
		Message: *req.Message,
		UserID:  (*req.User).ID,
		User:    *req.User,
	}
	return l.repo.Create(not)
}

func (l *NotificationLogic) DeleteNotification(id bson.ObjectID) error {
	return l.repo.Delete(id)
}
