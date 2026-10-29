package my_video

import (
	"fmt"
	"os"

	"security_camera/lib/config"
	"security_camera/lib/env"
)

// Pick returns the id of the camera to use. The choice is kept in CAMERA_ID;
// it is asked again only when refetch is set, nothing is saved yet or the
// saved camera is gone. With a single camera there is nothing to ask.
func Pick(refetch bool) (string, error) {
	devs := Devices()
	switch len(devs) {
	case 0:
		// let Open try the default camera and report the driver's own error
		fmt.Fprintln(os.Stderr, "no camera listed, trying the default one")
		return "", nil
	case 1:
		return devs[0].ID, nil
	}
	if saved := os.Getenv("CAMERA_ID"); saved != "" && !refetch {
		for _, d := range devs {
			if d.ID == saved {
				return saved, nil
			}
		}
		fmt.Println("saved camera not found, choosing again")
	}
	labels := make([]string, len(devs))
	for i, d := range devs {
		labels[i] = d.Label
	}
	chosen := devs[env.Choose(os.Stdin, os.Stdout, "Several cameras found:", labels, config.CameraChoiceTimeout)]
	if err := env.Save(config.EnvFile, "CAMERA_ID", chosen.ID); err != nil {
		return "", err
	}
	os.Setenv("CAMERA_ID", chosen.ID)
	return chosen.ID, nil
}
