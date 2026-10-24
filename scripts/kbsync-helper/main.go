// kbsync-helper wraps the pinned upstream StoreAgent guest implementation.
package main

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"strconv"
	"time"

	"github.com/majd/ipatool/v2/internal/sap/assets"
	"github.com/majd/ipatool/v2/internal/sap/machine"
)

type identity struct {
	hardware []byte
	dsid     uint64
}

func readIdentity(reader io.Reader) (identity, error) {
	const maxInput = 4096
	data, err := io.ReadAll(io.LimitReader(reader, maxInput+1))
	if err != nil || len(data) > maxInput {
		return identity{}, errors.New("input must be at most 4096 bytes")
	}
	var request struct {
		GUID string          `json:"guid"`
		DSID json.RawMessage `json:"dsid"`
	}
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&request); err != nil {
		return identity{}, errors.New("expected JSON object with guid and dsid only")
	}
	if err := decoder.Decode(new(any)); err != io.EOF {
		return identity{}, errors.New("expected exactly one JSON request")
	}
	hardware, err := hex.DecodeString(request.GUID)
	if err != nil || len(hardware) != 6 {
		return identity{}, errors.New("guid must be exactly 12 hexadecimal characters")
	}
	decimal := string(request.DSID)
	if len(decimal) > 0 && decimal[0] == '"' {
		if err := json.Unmarshal(request.DSID, &decimal); err != nil {
			return identity{}, errors.New("dsid must be an unsigned decimal integer")
		}
	}
	if len(decimal) == 0 {
		return identity{}, errors.New("dsid is required")
	}
	for _, char := range decimal {
		if char < '0' || char > '9' {
			return identity{}, errors.New("dsid must contain decimal digits only")
		}
	}
	dsid, err := strconv.ParseUint(decimal, 10, 64)
	if err != nil || dsid == 0 {
		return identity{}, errors.New("dsid must be a nonzero uint64")
	}
	return identity{hardware: hardware, dsid: dsid}, nil
}

type generator func(context.Context, []byte, uint64) ([]byte, error)

func run(ctx context.Context, input io.Reader, output io.Writer, generate generator) error {
	id, err := readIdentity(input)
	if err != nil {
		return err
	}
	blob, err := generate(ctx, id.hardware, id.dsid)
	if err != nil {
		return err
	}
	if len(blob) == 0 {
		return errors.New("generated kbsync is empty")
	}
	defer clear(blob)
	return json.NewEncoder(output).Encode(struct {
		KBSync string `json:"kbsync"`
	}{base64.StdEncoding.EncodeToString(blob)})
}

func generate(ctx context.Context, hardware []byte, dsid uint64) ([]byte, error) {
	bundle, err := assets.Load(ctx)
	if err != nil {
		return nil, fmt.Errorf("load pinned Apple assets: %w", err)
	}
	return machine.GenerateKBSync(ctx, bundle, hardware, dsid)
}

func main() {
	timeout := flag.Duration("timeout", 10*time.Minute, "overall asset loading and generation timeout; guest call limit remains upstream's one minute")
	flag.Parse()
	if flag.NArg() != 0 || *timeout <= 0 {
		fmt.Fprintln(os.Stderr, "kbsync-helper: supply identity on stdin; timeout must be positive")
		os.Exit(2)
	}
	ctx, cancel := context.WithTimeout(context.Background(), *timeout)
	defer cancel()
	if err := run(ctx, os.Stdin, os.Stdout, generate); err != nil {
		fmt.Fprintf(os.Stderr, "kbsync-helper: %v\n", err)
		os.Exit(1)
	}
}
