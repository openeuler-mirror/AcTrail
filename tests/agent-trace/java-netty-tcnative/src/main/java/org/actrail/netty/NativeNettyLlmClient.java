package org.actrail.netty;

import io.netty.bootstrap.Bootstrap;
import io.netty.buffer.ByteBuf;
import io.netty.buffer.Unpooled;
import io.netty.channel.Channel;
import io.netty.channel.ChannelFuture;
import io.netty.channel.ChannelHandlerContext;
import io.netty.channel.ChannelInitializer;
import io.netty.channel.ChannelOption;
import io.netty.channel.EventLoopGroup;
import io.netty.channel.SimpleChannelInboundHandler;
import io.netty.channel.nio.NioEventLoopGroup;
import io.netty.channel.socket.SocketChannel;
import io.netty.channel.socket.nio.NioSocketChannel;
import io.netty.handler.codec.http.DefaultFullHttpRequest;
import io.netty.handler.codec.http.FullHttpRequest;
import io.netty.handler.codec.http.FullHttpResponse;
import io.netty.handler.codec.http.HttpClientCodec;
import io.netty.handler.codec.http.HttpHeaderNames;
import io.netty.handler.codec.http.HttpHeaderValues;
import io.netty.handler.codec.http.HttpMethod;
import io.netty.handler.codec.http.HttpObjectAggregator;
import io.netty.handler.codec.http.HttpResponseStatus;
import io.netty.handler.codec.http.HttpVersion;
import io.netty.handler.ssl.OpenSsl;
import io.netty.handler.ssl.SslContext;
import io.netty.handler.ssl.SslContextBuilder;
import io.netty.handler.ssl.SslProvider;
import java.net.URI;
import java.nio.charset.StandardCharsets;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.concurrent.CompletableFuture;

public final class NativeNettyLlmClient {
    private static final String COMPLETE_MARKER = "ACTRAIL_NETTY_TCNATIVE_COMPLETE";

    private NativeNettyLlmClient() {}

    public static void main(String[] args) throws Exception {
        Settings settings = Settings.parse(args);
        if (!OpenSsl.isAvailable()) {
            throw new IllegalStateException("Netty OpenSSL provider unavailable", OpenSsl.unavailabilityCause());
        }
        URI uri = URI.create(settings.apiUrl);
        if (!"https".equalsIgnoreCase(uri.getScheme())) {
            throw new IllegalArgumentException("api-url must use https");
        }
        String apiKey = System.getenv(settings.apiKeyEnv);
        if (apiKey == null || apiKey.isBlank()) {
            throw new IllegalStateException("missing environment variable " + settings.apiKeyEnv);
        }
        System.out.println("netty_ssl_provider=OPENSSL");
        System.out.println("netty_openssl_version=" + OpenSsl.versionString());

        String body = requestBody(settings.model, settings.prompt);
        HttpResponse response = execute(uri, apiKey, body, settings);
        System.out.println("netty_http_status=" + response.status.code());
        System.out.println(response.body);
        if (response.status.code() >= settings.httpErrorStatusFloor) {
            throw new IllegalStateException("LLM provider returned " + response.status);
        }
        System.out.println(COMPLETE_MARKER);
    }

    private static HttpResponse execute(URI uri, String apiKey, String body, Settings settings)
            throws Exception {
        String host = uri.getHost();
        if (host == null || host.isBlank()) {
            throw new IllegalArgumentException("api-url must include host");
        }
        int port = uri.getPort() > 0 ? uri.getPort() : settings.defaultHttpsPort;
        String path = uri.getRawPath() == null || uri.getRawPath().isEmpty() ? "/" : uri.getRawPath();
        if (uri.getRawQuery() != null && !uri.getRawQuery().isEmpty()) {
            path = path + "?" + uri.getRawQuery();
        }

        EventLoopGroup group = new NioEventLoopGroup(settings.eventLoopThreads);
        CompletableFuture<HttpResponse> responseFuture = new CompletableFuture<>();
        try {
            SslContext sslContext = SslContextBuilder.forClient()
                    .sslProvider(SslProvider.OPENSSL)
                    .build();
            Bootstrap bootstrap = new Bootstrap()
                    .group(group)
                    .channel(NioSocketChannel.class)
                    .option(ChannelOption.CONNECT_TIMEOUT_MILLIS, settings.connectTimeoutMillis)
                    .handler(new ChannelInitializer<SocketChannel>() {
                        @Override
                        protected void initChannel(SocketChannel channel) {
                            channel.pipeline().addLast(sslContext.newHandler(channel.alloc(), host, port));
                            channel.pipeline().addLast(new HttpClientCodec());
                            channel.pipeline().addLast(new HttpObjectAggregator(settings.aggregationMaxBytes));
                            channel.pipeline().addLast(new ResponseHandler(responseFuture));
                        }
                    });

            Channel channel = bootstrap.connect(host, port).sync().channel();
            FullHttpRequest request = request(host, path, apiKey, body);
            ChannelFuture write = channel.writeAndFlush(request);
            write.sync();
            HttpResponse response = responseFuture.get();
            channel.closeFuture().sync();
            return response;
        } finally {
            group.shutdownGracefully().sync();
        }
    }

    private static FullHttpRequest request(String host, String path, String apiKey, String body) {
        ByteBuf content = Unpooled.copiedBuffer(body, StandardCharsets.UTF_8);
        FullHttpRequest request =
                new DefaultFullHttpRequest(HttpVersion.HTTP_1_1, HttpMethod.POST, path, content);
        request.headers().set(HttpHeaderNames.HOST, host);
        request.headers().set(HttpHeaderNames.AUTHORIZATION, "Bearer " + apiKey);
        request.headers().set(HttpHeaderNames.CONTENT_TYPE, HttpHeaderValues.APPLICATION_JSON);
        request.headers().set(HttpHeaderNames.ACCEPT, HttpHeaderValues.APPLICATION_JSON);
        request.headers().set(HttpHeaderNames.CONNECTION, HttpHeaderValues.CLOSE);
        request.headers().setInt(HttpHeaderNames.CONTENT_LENGTH, content.readableBytes());
        return request;
    }

    private static String requestBody(String model, String prompt) {
        return "{\"model\":\""
                + jsonString(model)
                + "\",\"messages\":[{\"role\":\"user\",\"content\":\""
                + jsonString(prompt)
                + "\"}],\"stream\":false}";
    }

    private static String jsonString(String value) {
        StringBuilder escaped = new StringBuilder(value.length());
        for (int index = 0; index < value.length(); index++) {
            char ch = value.charAt(index);
            switch (ch) {
                case '\\' -> escaped.append("\\\\");
                case '"' -> escaped.append("\\\"");
                case '\n' -> escaped.append("\\n");
                case '\r' -> escaped.append("\\r");
                case '\t' -> escaped.append("\\t");
                default -> escaped.append(ch);
            }
        }
        return escaped.toString();
    }

    private record HttpResponse(HttpResponseStatus status, String body) {}

    private static final class ResponseHandler extends SimpleChannelInboundHandler<FullHttpResponse> {
        private final CompletableFuture<HttpResponse> responseFuture;

        private ResponseHandler(CompletableFuture<HttpResponse> responseFuture) {
            this.responseFuture = responseFuture;
        }

        @Override
        protected void channelRead0(ChannelHandlerContext context, FullHttpResponse response) {
            String body = response.content().toString(StandardCharsets.UTF_8);
            responseFuture.complete(new HttpResponse(response.status(), body));
            context.close();
        }

        @Override
        public void exceptionCaught(ChannelHandlerContext context, Throwable cause) {
            responseFuture.completeExceptionally(cause);
            context.close();
        }
    }

    private static final class Settings {
        private final String apiUrl;
        private final String apiKeyEnv;
        private final String model;
        private final String prompt;
        private final int aggregationMaxBytes;
        private final int connectTimeoutMillis;
        private final int eventLoopThreads;
        private final int defaultHttpsPort;
        private final int httpErrorStatusFloor;

        private Settings(Map<String, String> values) {
            this.apiUrl = required(values, "--api-url");
            this.apiKeyEnv = required(values, "--api-key-env");
            this.model = required(values, "--model");
            this.prompt = required(values, "--prompt");
            this.aggregationMaxBytes = positiveInt(values, "--aggregation-max-bytes");
            this.connectTimeoutMillis = positiveInt(values, "--connect-timeout-ms");
            this.eventLoopThreads = positiveInt(values, "--event-loop-threads");
            this.defaultHttpsPort = positiveInt(values, "--default-https-port");
            this.httpErrorStatusFloor = positiveInt(values, "--http-error-status-floor");
        }

        private static Settings parse(String[] args) {
            if (args.length % 2 != 0) {
                throw new IllegalArgumentException("arguments must be --key value pairs");
            }
            Map<String, String> values = new LinkedHashMap<>();
            for (int index = 0; index < args.length; index += 2) {
                values.put(args[index], args[index + 1]);
            }
            return new Settings(values);
        }

        private static String required(Map<String, String> values, String key) {
            String value = values.get(key);
            if (value == null || value.isBlank()) {
                throw new IllegalArgumentException("missing " + key);
            }
            return value;
        }

        private static int positiveInt(Map<String, String> values, String key) {
            int value = Integer.parseInt(required(values, key));
            if (value <= 0) {
                throw new IllegalArgumentException(key + " must be positive");
            }
            return value;
        }
    }
}
