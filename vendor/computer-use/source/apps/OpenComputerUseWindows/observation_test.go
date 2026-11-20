package main

import (
	"encoding/json"
	"strings"
	"testing"
)

func TestObservationRequestSerializationAndChannels(t *testing.T) {
	for _, flags := range []map[string]any{{}, {"include_ax": false}, {"include_screenshot": false}, {"include_ax": true, "include_screenshot": true}} {
		s := newService()
		s.runBridge = func(request psRequest) (*psResponse, error) {
			data, err := json.Marshal(request)
			if err != nil {
				t.Fatal(err)
			}
			var sent map[string]any
			if err := json.Unmarshal(data, &sent); err != nil {
				t.Fatal(err)
			}
			for _, key := range []string{"include_ax", "include_screenshot"} {
				want, present := flags[key]
				got, exists := sent[key]
				if present != exists || want != got {
					t.Fatalf("%s serialized as %#v, want %#v", key, sent, flags)
				}
			}
			return &psResponse{OK: true, Snapshot: &appSnapshot{App: appDescriptor{Name: "Editor", PID: 42},
				WindowBounds: &frame{Width: 300, Height: 200}, ScreenshotPNGBase64: pngBase64(t, 300, 200),
				TreeLines: []string{"entry"}, FocusedSummary: "focus", SelectedText: "中文🙂", Elements: []elementRecord{{Index: 7}}}}, nil
		}
		args := map[string]any{"app": "Editor"}
		for key, value := range flags {
			args[key] = value
		}
		result := s.callTool("get_app_state", args)
		if result.IsError {
			t.Fatal(result)
		}
		if flags["include_screenshot"] == false {
			if len(result.Content) != 1 || !strings.Contains(result.Content[0].Text, "include_screenshot=false") {
				t.Fatal(result)
			}
		} else if len(result.Content) != 2 {
			t.Fatal(result)
		}
		if flags["include_ax"] == false {
			cached := s.currentSnapshot("Editor")
			if len(cached.Elements) != 0 || len(cached.TreeLines) != 0 || cached.FocusedSummary != "" || cached.SelectedText != "" {
				t.Fatal(cached)
			}
			if !strings.Contains(result.Content[0].Text, "indexes invalidated") {
				t.Fatal(result)
			}
		}
	}
}

func TestObservationRejectsInvalidOptionsBeforeBridge(t *testing.T) {
	s := newService()
	s.runBridge = func(psRequest) (*psResponse, error) { t.Fatal("invalid observation reached bridge"); return nil, nil }
	for _, args := range []map[string]any{{"include_ax": "false"}, {"include_screenshot": nil}, {"include_ax": 0}, {"include_ax": false, "include_screenshot": false}} {
		args["app"] = "Editor"
		if result := s.callTool("get_app_state", args); !result.IsError {
			t.Fatal(args)
		}
	}
}

func TestObservationInvalidatesOlderAliasesAcrossProcessReplacement(t *testing.T) {
	s := newService()
	s.rememberSnapshot("older-query", &appSnapshot{App: appDescriptor{Name: "Editor", PID: 42}, Elements: []elementRecord{{Index: 6}}})
	s.rememberSnapshot("Editor", &appSnapshot{App: appDescriptor{Name: "Editor", PID: 42}, Elements: []elementRecord{{Index: 7}}})
	other := &appSnapshot{App: appDescriptor{Name: "Other", PID: 99}}
	s.rememberSnapshot("Other", other)
	s.runBridge = func(request psRequest) (*psResponse, error) {
		return &psResponse{OK: true, Snapshot: &appSnapshot{App: appDescriptor{Name: "Editor", PID: 43},
			WindowBounds: &frame{Width: 300, Height: 200}, ScreenshotPNGBase64: pngBase64(t, 300, 200), Elements: []elementRecord{{Index: 8}}}}, nil
	}
	if result := s.callTool("get_app_state", map[string]any{"app": "Editor", "include_ax": false}); result.IsError {
		t.Fatal(result)
	}
	if s.currentSnapshot("older-query") != nil || s.currentSnapshot("42") != nil || s.currentSnapshot("Other") != other {
		t.Fatal("stale aliases retained or unrelated app invalidated")
	}
	if result := s.setValue("Editor", "7", "bad"); !result.IsError {
		t.Fatal("stale element action accepted")
	}
	x, y := 10.0, 20.0
	if result := s.click("Editor", "", &x, &y, 1, "left", "auto"); result.IsError {
		t.Fatal("screenshot-only coordinate action failed", result)
	}
	if result := s.callTool("get_app_state", map[string]any{"app": "Editor", "include_screenshot": false}); result.IsError {
		t.Fatal(result)
	}
	if _, err := lookupElement(s.currentSnapshot("Editor"), "8"); err != nil {
		t.Fatal("AX refresh did not restore indexes", err)
	}
	if result := s.click("Editor", "", &x, &y, 1, "left", "auto"); !result.IsError {
		t.Fatal("AX-only observation accepted screenshot coordinates")
	}
	if result := s.setValue("Editor", "8", "works"); result.IsError {
		t.Fatal("AX-only element action failed", result)
	}
}

func TestObservationSchemaDefaults(t *testing.T) {
	props := findToolDefinition(t, "get_app_state").InputSchema["properties"].(map[string]any)
	for _, key := range []string{"include_ax", "include_screenshot"} {
		value := props[key].(map[string]any)
		if value["type"] != "boolean" || value["default"] != true {
			t.Fatal(value)
		}
	}
}
