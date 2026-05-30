#!/usr/bin/env node
"use strict";

const https = require("https");

const DEFAULT_URL = process.env.ACTRAIL_NODE_HTTPS_URL || "https://example.com/";
const DEFAULT_DELAY_MS = process.env.ACTRAIL_NODE_HTTPS_DELAY_MS || "0";
const DEFAULT_HOLD_MS = process.env.ACTRAIL_NODE_HTTPS_HOLD_MS || "0";
const DEFAULT_METHOD = process.env.ACTRAIL_NODE_HTTPS_METHOD || "GET";

function usage() {
  console.error(`Usage: node tests/payload/claude-code/node_https_workload.js [options]

Options:
  --url URL          HTTPS URL to request. Default: ${DEFAULT_URL}
  --delay-ms MS     Wait before opening HTTPS. Default: ${DEFAULT_DELAY_MS}
  --hold-ms MS      Keep process alive after response end. Default: ${DEFAULT_HOLD_MS}
  --method METHOD   HTTP method. Default: ${DEFAULT_METHOD}
  --header K: V     Add a request header. Repeatable.
  --body TEXT       Send a UTF-8 request body.
  --help            Show this help.
`);
}

function parseArgs(argv) {
  const config = {
    url: DEFAULT_URL,
    delayMs: parseNonNegativeInt(DEFAULT_DELAY_MS, "ACTRAIL_NODE_HTTPS_DELAY_MS"),
    holdMs: parseNonNegativeInt(DEFAULT_HOLD_MS, "ACTRAIL_NODE_HTTPS_HOLD_MS"),
    method: DEFAULT_METHOD,
    headers: {},
    body: undefined,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--help") {
      usage();
      process.exit(0);
    }
    if (arg === "--url") {
      config.url = requiredValue(argv, ++index, arg);
    } else if (arg === "--delay-ms") {
      config.delayMs = parseNonNegativeInt(requiredValue(argv, ++index, arg), arg);
    } else if (arg === "--hold-ms") {
      config.holdMs = parseNonNegativeInt(requiredValue(argv, ++index, arg), arg);
    } else if (arg === "--method") {
      config.method = requiredValue(argv, ++index, arg);
    } else if (arg === "--header") {
      const [name, value] = parseHeader(requiredValue(argv, ++index, arg));
      config.headers[name] = value;
    } else if (arg === "--body") {
      config.body = requiredValue(argv, ++index, arg);
    } else {
      throw new Error(`unknown argument ${arg}`);
    }
  }
  return config;
}

function requiredValue(argv, index, flag) {
  const value = argv[index];
  if (!value) {
    throw new Error(`${flag} requires a value`);
  }
  return value;
}

function parseNonNegativeInt(raw, label) {
  if (!/^\d+$/.test(raw)) {
    throw new Error(`${label} must be a non-negative integer`);
  }
  return Number(raw);
}

function parseHeader(raw) {
  const separator = raw.indexOf(":");
  if (separator <= 0) {
    throw new Error(`invalid header ${raw}; expected "Name: value"`);
  }
  return [raw.slice(0, separator).trim(), raw.slice(separator + 1).trim()];
}

function run(config) {
  const body = config.body === undefined ? undefined : Buffer.from(config.body, "utf8");
  const url = new URL(config.url);
  if (url.protocol !== "https:") {
    throw new Error(`expected https URL, got ${config.url}`);
  }
  if (body && config.headers["content-length"] === undefined) {
    config.headers["content-length"] = String(body.byteLength);
  }

  console.log(
    `node_https_workload_started url=${url.href} delay_ms=${config.delayMs} hold_ms=${config.holdMs}`,
  );
  setTimeout(() => {
    const request = https.request(
      url,
      { method: config.method, headers: config.headers },
      (response) => {
        let bytes = 0;
        response.on("data", (chunk) => {
          bytes += chunk.length;
        });
        response.on("end", () => {
          console.log(
            `node_https_workload_done status=${response.statusCode} response_bytes=${bytes}`,
          );
          if (config.holdMs > 0) {
            console.log(`node_https_workload_hold ms=${config.holdMs}`);
            setTimeout(() => process.exit(0), config.holdMs);
          }
        });
      },
    );
    request.on("error", (error) => {
      console.error(`node_https_workload_error ${error.message}`);
      process.exit(2);
    });
    if (body) {
      request.write(body);
    }
    request.end();
  }, config.delayMs);
}

try {
  run(parseArgs(process.argv.slice(2)));
} catch (error) {
  console.error(error.message);
  usage();
  process.exit(1);
}
