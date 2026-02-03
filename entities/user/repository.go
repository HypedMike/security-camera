package user

import (
	"context"
	"security-camera/db"

	"go.mongodb.org/mongo-driver/v2/mongo/options"
)

type UserRepository struct {
	db             *db.DbStruct
	ctx            context.Context
	collectionName string
}

func NewUserRepository(db *db.DbStruct) *UserRepository {
	ctx := context.Background()
	return &UserRepository{
		db:             db,
		collectionName: "users",
		ctx:            ctx,
	}
}

func (ur *UserRepository) Find(filters map[string]interface{}) ([]User, error) {
	var users []User
	collection := ur.db.GetCollection(ur.collectionName)

	cursor, err := collection.Find(ur.ctx, filters)
	if err != nil {
		return nil, err
	}
	defer cursor.Close(ur.ctx)
	for cursor.Next(ur.ctx) {
		var user User
		if err := cursor.Decode(&user); err != nil {
			return nil, err
		}
		users = append(users, user)
	}

	if err := cursor.Err(); err != nil {
		return nil, err
	}

	return users, nil
}

func (ur *UserRepository) Insert(user *User) error {
	collection := ur.db.GetCollection(ur.collectionName)
	_, err := collection.InsertOne(ur.ctx, user)
	return err
}

func (ur *UserRepository) Upsert(filters map[string]interface{}, user *User) error {
	collection := ur.db.GetCollection(ur.collectionName)
	update := map[string]interface{}{
		"$set": user,
	}
	opts := options.UpdateOne().SetUpsert(true)
	_, err := collection.UpdateOne(ur.ctx, filters, update, opts)
	return err
}
