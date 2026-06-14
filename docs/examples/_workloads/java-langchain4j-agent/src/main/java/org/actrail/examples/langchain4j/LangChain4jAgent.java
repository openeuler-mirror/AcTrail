package org.actrail.examples.langchain4j;

import dev.langchain4j.model.chat.ChatModel;
import dev.langchain4j.model.openai.OpenAiChatModel;
import java.time.Duration;
import java.util.HashMap;
import java.util.Map;

public final class LangChain4jAgent {
    private LangChain4jAgent() {}

    public static void main(String[] args) {
        Config config = Config.parse(args);
        String apiKey = System.getenv(config.apiKeyEnv);
        if (apiKey == null || apiKey.isBlank()) {
            throw new IllegalStateException("missing environment variable " + config.apiKeyEnv);
        }

        Duration timeout = Duration.ofMillis(Math.round(config.requestTimeoutSeconds * 1000.0));
        ChatModel model = OpenAiChatModel.builder()
                .baseUrl(config.baseUrl)
                .apiKey(apiKey)
                .modelName(config.model)
                .temperature(0.0)
                .timeout(timeout)
                .maxRetries(0)
                .build();

        System.out.println("langchain4j_agent_start model=" + config.model + " base_url=" + config.baseUrl);
        String answer = model.chat(config.prompt);
        System.out.println("llm_answer=" + oneLine(answer));
        if (!config.expectedOutputFragment.isBlank() && !answer.contains(config.expectedOutputFragment)) {
            throw new IllegalStateException("LLM answer did not contain expected fragment");
        }
        System.out.println("ACTRAIL_LANGCHAIN4J_AGENT_COMPLETE");
    }

    private static String oneLine(String value) {
        return value.replace("\r", "\\r").replace("\n", "\\n");
    }

    private record Config(
            String prompt,
            String expectedOutputFragment,
            String model,
            String baseUrl,
            String apiKeyEnv,
            double requestTimeoutSeconds) {
        static Config parse(String[] args) {
            Map<String, String> values = parseFlags(args);
            double timeout = Double.parseDouble(required(values, "--request-timeout-seconds"));
            if (timeout <= 0.0) {
                throw new IllegalArgumentException("--request-timeout-seconds must be positive");
            }
            return new Config(
                    required(values, "--prompt"),
                    values.getOrDefault("--expected-output-fragment", ""),
                    required(values, "--model"),
                    required(values, "--base-url"),
                    required(values, "--api-key-env"),
                    timeout);
        }

        private static Map<String, String> parseFlags(String[] args) {
            Map<String, String> values = new HashMap<>();
            for (int i = 0; i < args.length; i += 2) {
                String key = args[i];
                if (!key.startsWith("--")) {
                    throw new IllegalArgumentException("expected flag, got " + key);
                }
                if (i + 1 >= args.length) {
                    throw new IllegalArgumentException("missing value for " + key);
                }
                values.put(key, args[i + 1]);
            }
            return values;
        }

        private static String required(Map<String, String> values, String key) {
            String value = values.get(key);
            if (value == null || value.isBlank()) {
                throw new IllegalArgumentException("missing required flag " + key);
            }
            return value;
        }
    }
}
