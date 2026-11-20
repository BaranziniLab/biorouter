package main

import (
	"strings"
	"testing"
)

func TestSnapshotObservationFlagsPreserveBudgetsAndBooleanTypes(t *testing.T) {
	args, flags, err := parseSnapshotObservationArgs([]string{"--include-screenshot", "false", "--max-tree-nodes", "2000", "Editor"})
	if err != nil {
		t.Fatal(err)
	}
	app, _, nodes, _, err := parseSnapshotArgs(args)
	if err != nil || app != "Editor" || nodes == nil || *nodes != 2000 || flags["include_screenshot"] != false {
		t.Fatalf("%v %v %v", args, flags, err)
	}
	for _, bad := range [][]string{{"--include-ax"}, {"--include-screenshot", "0"}, {"--include-ax", "no"}, {"--include-ax", "false", "--include-screenshot", "false"}} {
		if _, _, err := parseSnapshotObservationArgs(bad); err == nil {
			t.Fatalf("invalid flags accepted: %v", bad)
		}
	}
	_, defaults, err := parseSnapshotObservationArgs([]string{"Editor"})
	if err != nil || len(defaults) != 0 {
		t.Fatalf("legacy call changed: %v %v", defaults, err)
	}
	if !strings.Contains(helpText("snapshot"), "--include-screenshot") {
		t.Fatal("snapshot help omits observation flags")
	}
}
