package db

import (
	"context"

	"go.mongodb.org/mongo-driver/v2/mongo"
	"go.mongodb.org/mongo-driver/v2/mongo/options"
)

type DbStruct struct {
	uri          string
	client       *mongo.Client
	ctx          *context.Context
	databaseName string
}

func NewDb(uri string, name string) *DbStruct {
	// Defines the options for the MongoDB client
	opts := options.Client().ApplyURI(uri)
	// Creates a new client and connects to the server
	client, err := mongo.Connect(opts)
	if err != nil {
		panic(err)
	}

	ctx := context.Background()

	return &DbStruct{
		uri:          uri,
		client:       client,
		ctx:          &ctx,
		databaseName: name,
	}
}

func (dbs *DbStruct) Client() *mongo.Client {
	return dbs.client
}

func (dbs *DbStruct) Ctx() *context.Context {
	return dbs.ctx
}

func (dbs *DbStruct) GetCollection(collectionName string) *mongo.Collection {
	return dbs.client.Database(dbs.databaseName).Collection(collectionName)
}

func (dbs *DbStruct) GetUpsertOptions() *options.UpdateOneOptions {
	upsert := true
	return &options.UpdateOneOptions{
		Upsert: &upsert,
	}
}
