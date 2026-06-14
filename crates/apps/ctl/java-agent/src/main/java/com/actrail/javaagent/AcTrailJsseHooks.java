package com.actrail.javaagent;

import java.nio.ByteBuffer;
import javax.net.ssl.SSLEngineResult;

public final class AcTrailJsseHooks {
    private static final String OUTBOUND = "outbound";
    private static final String INBOUND = "inbound";
    private static final String ENGINE_WRAP = "jsse-engine-wrap";
    private static final String ENGINE_UNWRAP = "jsse-engine-unwrap";
    private static final String SOCKET_WRITE = "jsse-socket-write";
    private static final String SOCKET_READ = "jsse-socket-read";
    private static final String SOCKET_WRITE_BYTE = "jsse-socket-write-byte";
    private static final String SOCKET_READ_BYTE = "jsse-socket-read-byte";

    private AcTrailJsseHooks() {
    }

    public static int[] capturePositions(ByteBuffer[] buffers, int offset, int length) {
        if (!TlsSyncReporter.isEnabled() || buffers == null || offset < 0 || length < 0
                || offset > buffers.length || length > buffers.length - offset) {
            return null;
        }
        int[] positions = new int[length];
        for (int i = 0; i < length; i++) {
            ByteBuffer buffer = buffers[offset + i];
            positions[i] = buffer == null ? -1 : buffer.position();
        }
        return positions;
    }

    public static void afterEngineWrap(
            SSLEngineResult result,
            Object owner,
            ByteBuffer[] buffers,
            int offset,
            int length,
            int[] beforePositions) {
        if (result == null || result.bytesConsumed() <= 0) {
            return;
        }
        byte[] payload = copyAdvancedBytes(
                buffers,
                offset,
                length,
                beforePositions,
                result.bytesConsumed(),
                ENGINE_WRAP);
        TlsSyncReporter.reportPayload(OUTBOUND, ENGINE_WRAP, owner, payload);
    }

    public static void afterEngineUnwrap(
            SSLEngineResult result,
            Object owner,
            ByteBuffer[] buffers,
            int offset,
            int length,
            int[] beforePositions) {
        if (result == null || result.bytesProduced() <= 0) {
            return;
        }
        byte[] payload = copyAdvancedBytes(
                buffers,
                offset,
                length,
                beforePositions,
                result.bytesProduced(),
                ENGINE_UNWRAP);
        TlsSyncReporter.reportPayload(INBOUND, ENGINE_UNWRAP, owner, payload);
    }

    public static void afterSocketWrite(Object owner, byte[] buffer, int offset, int length) {
        byte[] payload = copyArrayBytes(buffer, offset, length, SOCKET_WRITE);
        TlsSyncReporter.reportPayload(OUTBOUND, SOCKET_WRITE, owner, payload);
    }

    public static void afterSocketWriteByte(Object owner, int value) {
        TlsSyncReporter.reportPayload(OUTBOUND, SOCKET_WRITE_BYTE, owner, new byte[] {(byte) value});
    }

    public static void afterSocketRead(int read, Object owner, byte[] buffer, int offset) {
        if (read <= 0) {
            return;
        }
        byte[] payload = copyArrayBytes(buffer, offset, read, SOCKET_READ);
        TlsSyncReporter.reportPayload(INBOUND, SOCKET_READ, owner, payload);
    }

    public static void afterSocketReadByte(int value, Object owner) {
        if (value < 0) {
            return;
        }
        TlsSyncReporter.reportPayload(INBOUND, SOCKET_READ_BYTE, owner, new byte[] {(byte) value});
    }

    private static byte[] copyAdvancedBytes(
            ByteBuffer[] buffers,
            int offset,
            int length,
            int[] beforePositions,
            int byteCount,
            String kind) {
        if (!TlsSyncReporter.isEnabled() || byteCount <= 0 || buffers == null || beforePositions == null
                || beforePositions.length != length || offset < 0 || length < 0
                || offset > buffers.length || length > buffers.length - offset) {
            return null;
        }
        if (byteCount > TlsSyncReporter.maxPayloadBytes()) {
            TlsSyncReporter.reportOverflow(kind, byteCount);
            return null;
        }
        byte[] payload = new byte[byteCount];
        int written = 0;
        for (int i = 0; i < length && written < byteCount; i++) {
            ByteBuffer buffer = buffers[offset + i];
            int before = beforePositions[i];
            if (buffer == null || before < 0) {
                continue;
            }
            int after = buffer.position();
            if (after <= before) {
                continue;
            }
            int take = Math.min(after - before, byteCount - written);
            ByteBuffer duplicate = buffer.asReadOnlyBuffer();
            duplicate.position(before);
            duplicate.limit(before + take);
            duplicate.get(payload, written, take);
            written += take;
        }
        if (written == payload.length) {
            return payload;
        }
        byte[] truncated = new byte[written];
        System.arraycopy(payload, 0, truncated, 0, written);
        return truncated;
    }

    private static byte[] copyArrayBytes(byte[] buffer, int offset, int length, String kind) {
        if (!TlsSyncReporter.isEnabled() || buffer == null || length <= 0 || offset < 0
                || offset > buffer.length || length > buffer.length - offset) {
            return null;
        }
        if (length > TlsSyncReporter.maxPayloadBytes()) {
            TlsSyncReporter.reportOverflow(kind, length);
            return null;
        }
        byte[] payload = new byte[length];
        System.arraycopy(buffer, offset, payload, 0, length);
        return payload;
    }
}
