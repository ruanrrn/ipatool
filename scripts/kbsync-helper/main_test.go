package main

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"strings"
	"testing"
)

func TestJSONOutputContract(t *testing.T) {
	var out bytes.Buffer
	err := run(context.Background(), strings.NewReader(`{"guid":"AABBCCDDEEFF","dsid":"123"}`), &out, func(ctx context.Context, hardware []byte, dsid uint64) ([]byte, error) {
		if len(hardware) != 6 || dsid != 123 {
			t.Fatal("wrong identity")
		}
		return []byte{0, 1, 2, 255}, nil
	})
	if err != nil {
		t.Fatal(err)
	}
	var result struct {
		KBSync string `json:"kbsync"`
	}
	if err := json.Unmarshal(out.Bytes(), &result); err != nil {
		t.Fatal(err)
	}
	decoded, err := base64.StdEncoding.DecodeString(result.KBSync)
	if err != nil || !bytes.Equal(decoded, []byte{0, 1, 2, 255}) {
		t.Fatal("bad output")
	}
}

func TestReadIdentity(t *testing.T) {
	for _, dsid := range []string{`"123456789"`, `123456789`, `"18446744073709551615"`} {
		id, err := readIdentity(strings.NewReader(`{"guid":"AABBCCDDEEFF","dsid":` + dsid + `}`))
		if err != nil {
			t.Fatal(err)
		}
		if hex.EncodeToString(id.hardware) != "aabbccddeeff" || id.dsid == 0 {
			t.Fatal("identity was not preserved")
		}
		if dsid == `"18446744073709551615"` && id.dsid != ^uint64(0) {
			t.Fatal("DSID precision lost")
		}
	}
}

func TestReadIdentityRejectsInvalidInput(t *testing.T) {
	for _, input := range []string{
		``, `{}`, `{"guid":"AA:BB:CC:DD:EE:FF","dsid":"123"}`,
		`{"guid":"AABBCCDDEEFG","dsid":"123"}`, `{"guid":"aabb","dsid":"123"}`,
		`{"guid":"AABBCCDDEEFF","dsid":"0"}`, `{"guid":"AABBCCDDEEFF","dsid":0}`,
		`{"guid":"AABBCCDDEEFF","dsid":"-1"}`, `{"guid":"AABBCCDDEEFF","dsid":"+1"}`,
		`{"guid":"AABBCCDDEEFF","dsid":1e3}`, `{"guid":"AABBCCDDEEFF","dsid":1.5}`,
		`{"guid":"AABBCCDDEEFF","dsid":null}`, `{"guid":"AABBCCDDEEFF","dsid":" 123"}`,
		`{"guid":"AABBCCDDEEFF","dsid":"18446744073709551616"}`,
		`{"guid":"AABBCCDDEEFF","dsid":"123","password":"must-not-read"}`,
		`{"guid":"AABBCCDDEEFF","dsid":"123"} {}`,
		strings.Repeat(" ", 4097) + `{}`,
	} {
		if _, err := readIdentity(strings.NewReader(input)); err == nil {
			t.Fatalf("accepted invalid input: %q", input)
		}
	}
}
