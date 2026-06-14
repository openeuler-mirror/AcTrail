package com.actrail.javaagent;

import java.net.StandardProtocolFamily;
import java.net.UnixDomainSocketAddress;
import java.nio.ByteBuffer;
import java.nio.channels.SocketChannel;
import java.nio.charset.StandardCharsets;
import java.util.Collections;
import java.util.Map;
import java.util.Optional;
import java.util.WeakHashMap;
import java.util.concurrent.atomic.AtomicLong;

final class TlsSyncReporter {
    private static final String ENV_TRACE_ID = "TLS_PAYLOAD_SYNC_TRACE_ID";
    private static final String ENV_EVENT_SOCKET = "TLS_PAYLOAD_SYNC_EVENT_SOCKET";
    private static final String ENV_MAX_PAYLOAD_BYTES = "TLS_PAYLOAD_SYNC_MAX_PAYLOAD_BYTES";
    private static final String PROVIDER = "jsse";
    private static final int DEFAULT_MAX_PAYLOAD_BYTES = 4 * 1024 * 1024;
    private static final char[] HEX = "0123456789abcdef".toCharArray();
    private static final Config CONFIG = Config.fromEnv();
    private static final Map<Object, Long> STREAM_KEYS =
            Collections.synchronizedMap(new WeakHashMap<>());
    private static final AtomicLong FALLBACK_STREAM_KEYS = new AtomicLong(1);
    private static final AtomicLong SEQUENCES = new AtomicLong(0);

    private TlsSyncReporter() {
    }

    static boolean isEnabled() {
        return CONFIG.enabled;
    }

    static int maxPayloadBytes() {
        return CONFIG.maxPayloadBytes;
    }

    static void reportPayload(String direction, String symbol, Object owner, byte[] payload) {
        if (!CONFIG.enabled || payload == null || payload.length == 0) {
            return;
        }
        if (payload.length > CONFIG.maxPayloadBytes) {
            reportOverflow(symbol, payload.length);
            return;
        }
        sendPayload(direction, symbol, streamKey(owner), payload);
    }

    static void reportOverflow(String kind, long bytes) {
        diagnostic(kind + " payload bytes=" + bytes
                + " exceeds " + ENV_MAX_PAYLOAD_BYTES + "=" + CONFIG.maxPayloadBytes
                + "; skipping report");
    }

    private static long streamKey(Object owner) {
        if (owner == null) {
            return FALLBACK_STREAM_KEYS.getAndIncrement();
        }
        synchronized (STREAM_KEYS) {
            Long existing = STREAM_KEYS.get(owner);
            if (existing != null) {
                return existing.longValue();
            }
            long next = FALLBACK_STREAM_KEYS.getAndIncrement();
            STREAM_KEYS.put(owner, next);
            return next;
        }
    }

    private static void sendPayload(String direction, String symbol, long streamKey, byte[] payload) {
        StringBuilder line = new StringBuilder(128 + payload.length * 2);
        line.append("v1\tpayload\t")
                .append(CONFIG.traceId)
                .append('\t')
                .append(CONFIG.pid)
                .append('\t')
                .append(direction)
                .append('\t')
                .append(PROVIDER)
                .append('\t')
                .append(symbol)
                .append('\t')
                .append(streamKey)
                .append('\t')
                .append(SEQUENCES.getAndIncrement())
                .append('\t');
        appendHex(line, payload);
        line.append('\n');
        byte[] encoded = line.toString().getBytes(StandardCharsets.UTF_8);
        try (SocketChannel channel = SocketChannel.open(StandardProtocolFamily.UNIX)) {
            channel.connect(UnixDomainSocketAddress.of(CONFIG.socketPath));
            ByteBuffer buffer = ByteBuffer.wrap(encoded);
            while (buffer.hasRemaining()) {
                channel.write(buffer);
            }
        } catch (Throwable error) {
            diagnostic("failed to send tls-sync payload event: " + error);
        }
    }

    private static void appendHex(StringBuilder output, byte[] bytes) {
        for (byte value : bytes) {
            output.append(HEX[(value >>> 4) & 0x0f]);
            output.append(HEX[value & 0x0f]);
        }
    }

    private static void diagnostic(String message) {
        System.err.println("actrail-java-payload-agent: " + message);
    }

    private static final class Config {
        private final boolean enabled;
        private final long traceId;
        private final long pid;
        private final String socketPath;
        private final int maxPayloadBytes;

        private Config(boolean enabled, long traceId, String socketPath, int maxPayloadBytes) {
            this.enabled = enabled;
            this.traceId = traceId;
            this.pid = ProcessHandle.current().pid();
            this.socketPath = socketPath;
            this.maxPayloadBytes = maxPayloadBytes;
        }

        private static Config fromEnv() {
            Optional<Long> traceId = parseLong(System.getenv(ENV_TRACE_ID));
            String socketPath = System.getenv(ENV_EVENT_SOCKET);
            int maxPayloadBytes = parsePositiveInt(
                    System.getenv(ENV_MAX_PAYLOAD_BYTES),
                    DEFAULT_MAX_PAYLOAD_BYTES);
            boolean enabled = traceId.isPresent() && socketPath != null && !socketPath.isBlank();
            return new Config(enabled, traceId.orElse(0L), socketPath, maxPayloadBytes);
        }

        private static Optional<Long> parseLong(String raw) {
            if (raw == null || raw.isBlank()) {
                return Optional.empty();
            }
            try {
                return Optional.of(Long.parseUnsignedLong(raw));
            } catch (NumberFormatException error) {
                diagnostic("invalid " + ENV_TRACE_ID + "=" + raw + "; Java payload capture disabled");
                return Optional.empty();
            }
        }

        private static int parsePositiveInt(String raw, int fallback) {
            if (raw == null || raw.isBlank()) {
                return fallback;
            }
            try {
                int value = Integer.parseInt(raw);
                return value > 0 ? value : fallback;
            } catch (NumberFormatException error) {
                diagnostic("invalid " + ENV_MAX_PAYLOAD_BYTES + "=" + raw + "; using " + fallback);
                return fallback;
            }
        }
    }
}
