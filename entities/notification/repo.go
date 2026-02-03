package notification

import (
	"security-camera/db"

	"go.mongodb.org/mongo-driver/v2/bson"
)

type NotificationRepo struct {
	dbStruct *db.DbStruct
	repoName string
}

func NewNotificationRepo(dbStruct *db.DbStruct) *NotificationRepo {
	return &NotificationRepo{
		dbStruct: dbStruct,
		repoName: "notifications",
	}
}

func (r *NotificationRepo) GetAll() ([]Notification, error) {
	cursor, err := r.dbStruct.GetCollection(r.repoName).Find(*r.dbStruct.Ctx(), map[string]interface{}{})
	if err != nil {
		return nil, err
	}
	var notifications []Notification
	if err = cursor.All(*r.dbStruct.Ctx(), &notifications); err != nil {
		return nil, err
	}
	return notifications, nil
}

func (r *NotificationRepo) Create(not Notification) error {
	_, err := r.dbStruct.GetCollection(r.repoName).InsertOne(*r.dbStruct.Ctx(), not)
	return err
}

func (r *NotificationRepo) Delete(id bson.ObjectID) error {
	_, err := r.dbStruct.GetCollection(r.repoName).DeleteOne(*r.dbStruct.Ctx(), bson.M{"_id": id})
	return err
}
