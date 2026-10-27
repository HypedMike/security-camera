package storage

func Init(root string) (IStorage, error) {
	return NewLocalStorage(root)
}
