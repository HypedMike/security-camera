package storage

type IStorage interface {
	Save(path string, data []byte) error
	Delete(path string) error
	Get(path string) ([]byte, error)
}
