package com.actrail.javaagent;

import java.lang.instrument.ClassFileTransformer;
import java.lang.instrument.IllegalClassFormatException;
import java.security.ProtectionDomain;
import java.util.Set;

public final class JsseTransformer implements ClassFileTransformer {
    static final String ENGINE = "sun/security/ssl/SSLEngineImpl";
    static final String SOCKET_OUTPUT = "sun/security/ssl/SSLSocketImpl$AppOutputStream";
    static final String SOCKET_INPUT = "sun/security/ssl/SSLSocketImpl$AppInputStream";
    private static final Set<String> TARGETS = Set.of(ENGINE, SOCKET_OUTPUT, SOCKET_INPUT);

    public JsseTransformer() {
    }

    @Override
    public byte[] transform(
            Module module,
            ClassLoader loader,
            String className,
            Class<?> classBeingRedefined,
            ProtectionDomain protectionDomain,
            byte[] classfileBuffer) throws IllegalClassFormatException {
        if (!TARGETS.contains(className)) {
            return null;
        }
        try {
            byte[] patched = ClassFileJssePatcher.patch(className, classfileBuffer);
            if (patched == null) {
                AcTrailJavaPayloadAgent.diagnostic("JSSE class shape was not recognized: " + className);
            }
            return patched;
        } catch (Throwable error) {
            AcTrailJavaPayloadAgent.diagnostic("JSSE transform failed for " + className + ": " + error);
            return null;
        }
    }
}
