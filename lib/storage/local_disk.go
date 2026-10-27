package storage

import (
	"fmt"
	"os"
	"path/filepath"
)

// LocalStorage stores files under a root directory on the local disk.
type LocalStorage struct {
	root string
}

func NewLocalStorage(root string) (*LocalStorage, error) {
	if err := os.MkdirAll(root, 0o755); err != nil {
		return nil, err
	}
	return &LocalStorage{root: root}, nil
}

// resolve maps a relative path into the root, rejecting paths that escape it.
func (ls *LocalStorage) resolve(path string) (string, error) {
	if path == "" {
		return "", fmt.Errorf("storage: empty path")
	}
	if filepath.IsAbs(path) || !filepath.IsLocal(path) {
		return "", fmt.Errorf("storage: invalid path %q", path)
	}
	return filepath.Join(ls.root, path), nil
}

// Save writes data to path, creating parent directories. The write is atomic:
// data goes to a temp file first and is renamed into place.
func (ls *LocalStorage) Save(path string, data []byte) error {
	full, err := ls.resolve(path)
	if err != nil {
		return err
	}
	dir := filepath.Dir(full)
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return err
	}
	tmp, err := os.CreateTemp(dir, ".tmp-*")
	if err != nil {
		return err
	}
	defer os.Remove(tmp.Name())
	if _, err := tmp.Write(data); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Close(); err != nil {
		return err
	}
	return os.Rename(tmp.Name(), full)
}

func (ls *LocalStorage) Get(path string) ([]byte, error) {
	full, err := ls.resolve(path)
	if err != nil {
		return nil, err
	}
	return os.ReadFile(full)
}

// Delete removes the file at path. Deleting a missing file is not an error.
func (ls *LocalStorage) Delete(path string) error {
	full, err := ls.resolve(path)
	if err != nil {
		return err
	}
	if err := os.Remove(full); err != nil && !os.IsNotExist(err) {
		return err
	}
	return nil
}
