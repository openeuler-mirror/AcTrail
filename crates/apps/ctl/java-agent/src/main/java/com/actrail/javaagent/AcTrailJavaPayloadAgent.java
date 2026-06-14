package com.actrail.javaagent;

import java.io.File;
import java.lang.instrument.Instrumentation;
import java.net.URI;
import java.security.CodeSource;
import java.util.Set;
import java.util.jar.JarFile;

public final class AcTrailJavaPayloadAgent {
    private static final String HOOK_CLASS = "com.actrail.javaagent.AcTrailJsseHooks";
    private static final Set<String> TARGET_CLASSES = Set.of(
            "sun.security.ssl.SSLEngineImpl",
            "sun.security.ssl.SSLSocketImpl$AppOutputStream",
            "sun.security.ssl.SSLSocketImpl$AppInputStream");

    private AcTrailJavaPayloadAgent() {
    }

    public static void premain(String agentArgs, Instrumentation instrumentation) {
        try {
            if (!prepareBootstrapHooks(instrumentation)) {
                return;
            }
            JsseTransformer transformer = new JsseTransformer();
            boolean canRetransform = instrumentation.isRetransformClassesSupported();
            instrumentation.addTransformer(transformer, canRetransform);
            if (canRetransform) {
                retransformAlreadyLoadedTargets(instrumentation);
            }
            diagnostic("installed JSSE payload transformer");
        } catch (Throwable error) {
            diagnostic("failed to install JSSE payload transformer: " + error);
        }
    }

    private static boolean prepareBootstrapHooks(Instrumentation instrumentation) {
        try {
            CodeSource source = AcTrailJavaPayloadAgent.class.getProtectionDomain().getCodeSource();
            if (source == null || source.getLocation() == null) {
                diagnostic("agent code source is unavailable; JSSE capture disabled");
                return false;
            }
            URI uri = source.getLocation().toURI();
            instrumentation.appendToBootstrapClassLoaderSearch(new JarFile(new File(uri)));
            Class.forName(HOOK_CLASS, false, null);
            return true;
        } catch (Throwable error) {
            diagnostic("cannot append agent to bootstrap class loader: " + error);
            return false;
        }
    }

    private static void retransformAlreadyLoadedTargets(Instrumentation instrumentation) {
        for (Class<?> loaded : instrumentation.getAllLoadedClasses()) {
            if (!TARGET_CLASSES.contains(loaded.getName())) {
                continue;
            }
            if (!instrumentation.isModifiableClass(loaded)) {
                diagnostic("JSSE class is already loaded and not modifiable: " + loaded.getName());
                continue;
            }
            try {
                instrumentation.retransformClasses(loaded);
            } catch (Throwable error) {
                diagnostic("cannot retransform already loaded JSSE class "
                        + loaded.getName() + ": " + error);
            }
        }
    }

    static void diagnostic(String message) {
        System.err.println("actrail-java-payload-agent: " + message);
    }
}
