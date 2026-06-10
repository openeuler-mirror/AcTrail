package main

import (
	"bytes"
	"crypto/tls"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"net/http"
	"os"
	"time"
)

func main() {
	apiURL := flag.String("api-url", "", "OpenAI-compatible chat completions URL")
	apiKeyEnv := flag.String("api-key-env", "", "environment variable containing the API key")
	model := flag.String("model", "", "model name")
	prompt := flag.String("prompt", "", "user prompt")
	flag.Parse()
	requireFlag("api-url", *apiURL)
	requireFlag("api-key-env", *apiKeyEnv)
	requireFlag("model", *model)
	requireFlag("prompt", *prompt)
	apiKey := os.Getenv(*apiKeyEnv)
	if apiKey == "" {
		fmt.Fprintf(os.Stderr, "%s is required\n", *apiKeyEnv)
		os.Exit(2)
	}
	body, err := json.Marshal(map[string]any{
		"model": *model,
		"messages": []map[string]string{
			{"role": "user", "content": *prompt},
		},
		"stream": false,
	})
	if err != nil {
		panic(err)
	}
	transport := &http.Transport{
		TLSNextProto: map[string]func(string, *tls.Conn) http.RoundTripper{},
	}
	client := &http.Client{
		Transport: transport,
		Timeout:   30 * time.Second,
	}
	request, err := http.NewRequest(http.MethodPost, *apiURL, bytes.NewReader(body))
	if err != nil {
		panic(err)
	}
	request.Header.Set("Authorization", "Bearer "+apiKey)
	request.Header.Set("Content-Type", "application/json")
	response, err := client.Do(request)
	if err != nil {
		panic(err)
	}
	defer response.Body.Close()
	responseBody, err := io.ReadAll(response.Body)
	if err != nil {
		panic(err)
	}
	fmt.Printf("go-stdlib-status=%s bytes=%d\n", response.Status, len(responseBody))
}

func requireFlag(name string, value string) {
	if value == "" {
		fmt.Fprintf(os.Stderr, "-%s is required\n", name)
		os.Exit(2)
	}
}
