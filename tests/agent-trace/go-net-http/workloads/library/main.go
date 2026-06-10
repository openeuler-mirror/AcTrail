package main

import (
	"flag"
	"fmt"
	"os"

	"actrail-go-library-workload/llmclient"
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
	status, size, err := llmclient.ChatCompletion(*apiURL, apiKey, *model, *prompt)
	if err != nil {
		panic(err)
	}
	fmt.Printf("go-library-status=%s bytes=%d\n", status, size)
}

func requireFlag(name string, value string) {
	if value == "" {
		fmt.Fprintf(os.Stderr, "-%s is required\n", name)
		os.Exit(2)
	}
}
