package com.actrail.javaagent;

import java.io.ByteArrayOutputStream;
import java.io.DataOutputStream;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;

final class ClassFileJssePatcher {
    private static final int MAGIC = 0xCAFEBABE;
    private static final String CODE = "Code";
    private static final String EXCEPTIONS = "Exceptions";
    private static final String HOOK_CLASS = "com/actrail/javaagent/AcTrailJsseHooks";
    private static final String BYTE_BUFFER_ARRAY = "[Ljava/nio/ByteBuffer;";
    private static final String SSL_ENGINE_RESULT = "Ljavax/net/ssl/SSLEngineResult;";
    private static final String CAPTURE_POSITIONS = "capturePositions";
    private static final String AFTER_ENGINE_WRAP = "afterEngineWrap";
    private static final String AFTER_ENGINE_UNWRAP = "afterEngineUnwrap";
    private static final String AFTER_SOCKET_WRITE = "afterSocketWrite";
    private static final String AFTER_SOCKET_WRITE_BYTE = "afterSocketWriteByte";
    private static final String AFTER_SOCKET_READ = "afterSocketRead";
    private static final String AFTER_SOCKET_READ_BYTE = "afterSocketReadByte";
    private static final String CAPTURE_POSITIONS_DESCRIPTOR = "(" + BYTE_BUFFER_ARRAY + "II)[I";
    private static final String ENGINE_HOOK_DESCRIPTOR =
            "(" + SSL_ENGINE_RESULT + "Ljava/lang/Object;" + BYTE_BUFFER_ARRAY + "II[I)V";
    private static final String SOCKET_WRITE_DESCRIPTOR = "(Ljava/lang/Object;[BII)V";
    private static final String SOCKET_WRITE_BYTE_DESCRIPTOR = "(Ljava/lang/Object;I)V";
    private static final String SOCKET_READ_DESCRIPTOR = "(ILjava/lang/Object;[BI)V";
    private static final String SOCKET_READ_BYTE_DESCRIPTOR = "(ILjava/lang/Object;)V";

    private ClassFileJssePatcher() {
    }

    static byte[] patch(String className, byte[] input) throws IOException {
        PatchSpec spec = PatchSpec.forClass(className);
        if (spec == null || u4(input, 0) != MAGIC) {
            return null;
        }
        int constantPoolCount = u2(input, 8);
        ConstantPool constantPool = readConstantPool(input, constantPoolCount);
        AddedConstants added = new AddedConstants(constantPoolCount, className, spec);

        ByteArrayOutputStream bytes = new ByteArrayOutputStream(input.length + 512);
        DataOutputStream out = new DataOutputStream(bytes);
        out.write(input, 0, 8);
        out.writeShort(constantPoolCount + added.count());
        out.write(input, 10, constantPool.endOffset - 10);
        added.write(out);

        int offset = constantPool.endOffset;
        int classHeaderStart = offset;
        offset += 6;
        int interfaceCount = u2(input, offset);
        offset += 2 + interfaceCount * 2;
        int fieldsCount = u2(input, offset);
        offset += 2;
        for (int i = 0; i < fieldsCount; i++) {
            offset = skipMember(input, offset);
        }
        out.write(input, classHeaderStart, offset - classHeaderStart);

        int methodsCount = u2(input, offset);
        offset += 2;
        ByteArrayOutputStream methodBytes = new ByteArrayOutputStream();
        DataOutputStream methodsOut = new DataOutputStream(methodBytes);
        List<WrapperMethod> wrappers = new ArrayList<>();
        for (int i = 0; i < methodsCount; i++) {
            MethodInfo method = MethodInfo.read(input, offset, constantPool);
            Target target = spec.target(method.name, method.descriptor);
            if (target == null) {
                methodsOut.write(input, method.startOffset, method.endOffset - method.startOffset);
            } else {
                method.writeRenamed(methodsOut, input, target.renamedNameIndex);
                wrappers.add(new WrapperMethod(method, target));
            }
            offset = method.endOffset;
        }
        if (wrappers.isEmpty()) {
            return null;
        }

        out.writeShort(methodsCount + wrappers.size());
        out.write(methodBytes.toByteArray());
        for (WrapperMethod wrapper : wrappers) {
            wrapper.write(out, added, input);
        }
        out.write(input, offset, input.length - offset);
        return bytes.toByteArray();
    }

    private static void writeWrapperCode(
            ByteArrayOutputStream code,
            AddedConstants added,
            Target target) {
        switch (target.kind) {
            case ENGINE_WRAP:
                writeEngineCapture(code, added, target.captureArrayLocal, target.captureOffsetLocal, target.captureLengthLocal);
                writeAStore(code, target.positionsLocal());
                writeOriginalCall(code, target);
                code.write(0x59);
                writeALoad(code, 0);
                writeALoad(code, target.captureArrayLocal);
                writeILoad(code, target.captureOffsetLocal);
                writeILoad(code, target.captureLengthLocal);
                writeALoad(code, target.positionsLocal());
                writeInvokeStatic(code, added.afterEngineWrapMethodRef);
                code.write(0xb0);
                break;
            case ENGINE_UNWRAP:
                writeEngineCapture(code, added, target.captureArrayLocal, target.captureOffsetLocal, target.captureLengthLocal);
                writeAStore(code, target.positionsLocal());
                writeOriginalCall(code, target);
                code.write(0x59);
                writeALoad(code, 0);
                writeALoad(code, target.captureArrayLocal);
                writeILoad(code, target.captureOffsetLocal);
                writeILoad(code, target.captureLengthLocal);
                writeALoad(code, target.positionsLocal());
                writeInvokeStatic(code, added.afterEngineUnwrapMethodRef);
                code.write(0xb0);
                break;
            case SOCKET_WRITE:
                writeOriginalCall(code, target);
                writeALoad(code, 0);
                writeALoad(code, 1);
                writeILoad(code, 2);
                writeILoad(code, 3);
                writeInvokeStatic(code, added.afterSocketWriteMethodRef);
                code.write(0xb1);
                break;
            case SOCKET_WRITE_BYTE:
                writeOriginalCall(code, target);
                writeALoad(code, 0);
                writeILoad(code, 1);
                writeInvokeStatic(code, added.afterSocketWriteByteMethodRef);
                code.write(0xb1);
                break;
            case SOCKET_READ:
                writeOriginalCall(code, target);
                code.write(0x59);
                writeALoad(code, 0);
                writeALoad(code, 1);
                writeILoad(code, 2);
                writeInvokeStatic(code, added.afterSocketReadMethodRef);
                code.write(0xac);
                break;
            case SOCKET_READ_BYTE:
                writeOriginalCall(code, target);
                code.write(0x59);
                writeALoad(code, 0);
                writeInvokeStatic(code, added.afterSocketReadByteMethodRef);
                code.write(0xac);
                break;
            default:
                throw new AssertionError(target.kind);
        }
    }

    private static void writeEngineCapture(
            ByteArrayOutputStream code,
            AddedConstants added,
            int arrayLocal,
            int offsetLocal,
            int lengthLocal) {
        writeALoad(code, arrayLocal);
        writeILoad(code, offsetLocal);
        writeILoad(code, lengthLocal);
        writeInvokeStatic(code, added.capturePositionsMethodRef);
    }

    private static void writeOriginalCall(ByteArrayOutputStream code, Target target) {
        writeALoad(code, 0);
        switch (target.argumentShape) {
            case ENGINE_TWO_ARRAYS:
                writeALoad(code, 1);
                writeILoad(code, 2);
                writeILoad(code, 3);
                writeALoad(code, 4);
                writeILoad(code, 5);
                writeILoad(code, 6);
                break;
            case BYTE_ARRAY_OFFSET_LENGTH:
                writeALoad(code, 1);
                writeILoad(code, 2);
                writeILoad(code, 3);
                break;
            case INT_VALUE:
                writeILoad(code, 1);
                break;
            case NO_ARGS:
                break;
            default:
                throw new AssertionError(target.argumentShape);
        }
        writeInvokeVirtual(code, target.renamedMethodRef);
    }

    private static void writeALoad(ByteArrayOutputStream out, int local) {
        if (local >= 0 && local <= 3) {
            out.write(0x2a + local);
        } else {
            out.write(0x19);
            out.write(local);
        }
    }

    private static void writeAStore(ByteArrayOutputStream out, int local) {
        if (local >= 0 && local <= 3) {
            out.write(0x4b + local);
        } else {
            out.write(0x3a);
            out.write(local);
        }
    }

    private static void writeILoad(ByteArrayOutputStream out, int local) {
        if (local >= 0 && local <= 3) {
            out.write(0x1a + local);
        } else {
            out.write(0x15);
            out.write(local);
        }
    }

    private static void writeInvokeStatic(ByteArrayOutputStream out, int methodRef) {
        writeMemberRef(out, 0xb8, methodRef);
    }

    private static void writeInvokeVirtual(ByteArrayOutputStream out, int methodRef) {
        writeMemberRef(out, 0xb6, methodRef);
    }

    private static void writeMemberRef(ByteArrayOutputStream out, int opcode, int ref) {
        out.write(opcode);
        out.write((ref >>> 8) & 0xff);
        out.write(ref & 0xff);
    }

    private static ConstantPool readConstantPool(byte[] input, int count) {
        Entry[] entries = new Entry[count];
        int offset = 10;
        for (int index = 1; index < count; index++) {
            int tag = input[offset++] & 0xff;
            Entry entry = new Entry(tag);
            switch (tag) {
                case 1:
                    int length = u2(input, offset);
                    offset += 2;
                    entry.utf8 = new String(input, offset, length, StandardCharsets.UTF_8);
                    offset += length;
                    break;
                case 3:
                case 4:
                    offset += 4;
                    break;
                case 5:
                case 6:
                    offset += 8;
                    entries[index] = entry;
                    index++;
                    continue;
                case 7:
                case 8:
                case 16:
                case 19:
                case 20:
                    entry.a = u2(input, offset);
                    offset += 2;
                    break;
                case 9:
                case 10:
                case 11:
                case 12:
                case 17:
                case 18:
                    entry.a = u2(input, offset);
                    entry.b = u2(input, offset + 2);
                    offset += 4;
                    break;
                case 15:
                    entry.a = input[offset] & 0xff;
                    entry.b = u2(input, offset + 1);
                    offset += 3;
                    break;
                default:
                    throw new IllegalArgumentException("unknown constant pool tag " + tag);
            }
            entries[index] = entry;
        }
        return new ConstantPool(entries, offset);
    }

    private static int skipMember(byte[] input, int offset) {
        int attributes = u2(input, offset + 6);
        offset += 8;
        for (int i = 0; i < attributes; i++) {
            int length = u4(input, offset + 2);
            offset += 6 + length;
        }
        return offset;
    }

    private static void writeUtf8(DataOutputStream out, String value) throws IOException {
        byte[] bytes = value.getBytes(StandardCharsets.UTF_8);
        out.writeByte(1);
        out.writeShort(bytes.length);
        out.write(bytes);
    }

    private static void writeClass(DataOutputStream out, int nameIndex) throws IOException {
        out.writeByte(7);
        out.writeShort(nameIndex);
    }

    private static void writeNameAndType(DataOutputStream out, int nameIndex, int descriptorIndex)
            throws IOException {
        out.writeByte(12);
        out.writeShort(nameIndex);
        out.writeShort(descriptorIndex);
    }

    private static void writeMethodRef(DataOutputStream out, int classIndex, int nameAndTypeIndex)
            throws IOException {
        out.writeByte(10);
        out.writeShort(classIndex);
        out.writeShort(nameAndTypeIndex);
    }

    private static int u2(byte[] input, int offset) {
        return ((input[offset] & 0xff) << 8) | (input[offset + 1] & 0xff);
    }

    private static int u4(byte[] input, int offset) {
        return ((input[offset] & 0xff) << 24)
                | ((input[offset + 1] & 0xff) << 16)
                | ((input[offset + 2] & 0xff) << 8)
                | (input[offset + 3] & 0xff);
    }

    private enum Kind {
        ENGINE_WRAP,
        ENGINE_UNWRAP,
        SOCKET_WRITE,
        SOCKET_WRITE_BYTE,
        SOCKET_READ,
        SOCKET_READ_BYTE
    }

    private enum ArgumentShape {
        ENGINE_TWO_ARRAYS,
        BYTE_ARRAY_OFFSET_LENGTH,
        INT_VALUE,
        NO_ARGS
    }

    private static final class PatchSpec {
        private final Target[] targets;

        private PatchSpec(Target[] targets) {
            this.targets = targets;
        }

        private static PatchSpec forClass(String className) {
            if (JsseTransformer.ENGINE.equals(className)) {
                return new PatchSpec(new Target[] {
                        Target.engineWrap(
                                "wrap",
                                "([Ljava/nio/ByteBuffer;II[Ljava/nio/ByteBuffer;II)Ljavax/net/ssl/SSLEngineResult;",
                                "actrail$wrapManyDestinations",
                                ArgumentShape.ENGINE_TWO_ARRAYS,
                                7,
                                1,
                                2,
                                3,
                                7),
                        Target.engineUnwrap(
                                "unwrap",
                                "([Ljava/nio/ByteBuffer;II[Ljava/nio/ByteBuffer;II)Ljavax/net/ssl/SSLEngineResult;",
                                "actrail$unwrapManySources",
                                ArgumentShape.ENGINE_TWO_ARRAYS,
                                7,
                                4,
                                5,
                                6,
                                7)
                });
            }
            if (JsseTransformer.SOCKET_OUTPUT.equals(className)) {
                return new PatchSpec(new Target[] {
                        Target.socket(
                                "write",
                                "([BII)V",
                                "actrail$writeBytes",
                                Kind.SOCKET_WRITE,
                                ArgumentShape.BYTE_ARRAY_OFFSET_LENGTH,
                                4,
                                4),
                        Target.socket(
                                "write",
                                "(I)V",
                                "actrail$writeByte",
                                Kind.SOCKET_WRITE_BYTE,
                                ArgumentShape.INT_VALUE,
                                2,
                                2)
                });
            }
            if (JsseTransformer.SOCKET_INPUT.equals(className)) {
                return new PatchSpec(new Target[] {
                        Target.socket(
                                "read",
                                "([BII)I",
                                "actrail$readBytes",
                                Kind.SOCKET_READ,
                                ArgumentShape.BYTE_ARRAY_OFFSET_LENGTH,
                                4,
                                4),
                        Target.socket(
                                "read",
                                "()I",
                                "actrail$readByte",
                                Kind.SOCKET_READ_BYTE,
                                ArgumentShape.NO_ARGS,
                                1,
                                2)
                });
            }
            return null;
        }

        private Target target(String name, String descriptor) {
            for (Target target : targets) {
                if (target.name.equals(name) && target.descriptor.equals(descriptor)) {
                    return target;
                }
            }
            return null;
        }
    }

    private static final class Target {
        private final String name;
        private final String descriptor;
        private final String renamedName;
        private final Kind kind;
        private final ArgumentShape argumentShape;
        private final int baseLocals;
        private final int captureArrayLocal;
        private final int captureOffsetLocal;
        private final int captureLengthLocal;
        private final int maxStack;
        private int renamedNameIndex;
        private int renamedMethodRef;

        private Target(
                String name,
                String descriptor,
                String renamedName,
                Kind kind,
                ArgumentShape argumentShape,
                int baseLocals,
                int captureArrayLocal,
                int captureOffsetLocal,
                int captureLengthLocal,
                int maxStack) {
            this.name = name;
            this.descriptor = descriptor;
            this.renamedName = renamedName;
            this.kind = kind;
            this.argumentShape = argumentShape;
            this.baseLocals = baseLocals;
            this.captureArrayLocal = captureArrayLocal;
            this.captureOffsetLocal = captureOffsetLocal;
            this.captureLengthLocal = captureLengthLocal;
            this.maxStack = maxStack;
        }

        private static Target engineWrap(
                String name,
                String descriptor,
                String renamedName,
                ArgumentShape argumentShape,
                int baseLocals,
                int captureArrayLocal,
                int captureOffsetLocal,
                int captureLengthLocal,
                int maxStack) {
            return new Target(
                    name,
                    descriptor,
                    renamedName,
                    Kind.ENGINE_WRAP,
                    argumentShape,
                    baseLocals,
                    captureArrayLocal,
                    captureOffsetLocal,
                    captureLengthLocal,
                    maxStack);
        }

        private static Target engineUnwrap(
                String name,
                String descriptor,
                String renamedName,
                ArgumentShape argumentShape,
                int baseLocals,
                int captureArrayLocal,
                int captureOffsetLocal,
                int captureLengthLocal,
                int maxStack) {
            return new Target(
                    name,
                    descriptor,
                    renamedName,
                    Kind.ENGINE_UNWRAP,
                    argumentShape,
                    baseLocals,
                    captureArrayLocal,
                    captureOffsetLocal,
                    captureLengthLocal,
                    maxStack);
        }

        private static Target socket(
                String name,
                String descriptor,
                String renamedName,
                Kind kind,
                ArgumentShape argumentShape,
                int baseLocals,
                int maxStack) {
            return new Target(name, descriptor, renamedName, kind, argumentShape, baseLocals, 0, 0, 0, maxStack);
        }

        private int positionsLocal() {
            return baseLocals;
        }

        private int maxLocals() {
            return kind == Kind.ENGINE_WRAP || kind == Kind.ENGINE_UNWRAP ? baseLocals + 1 : baseLocals;
        }
    }

    private static final class AddedConstants {
        private final List<ConstantWriter> entries = new ArrayList<>();
        private int next;
        private final int codeName;
        private final int capturePositionsMethodRef;
        private final int afterEngineWrapMethodRef;
        private final int afterEngineUnwrapMethodRef;
        private final int afterSocketWriteMethodRef;
        private final int afterSocketWriteByteMethodRef;
        private final int afterSocketReadMethodRef;
        private final int afterSocketReadByteMethodRef;

        private AddedConstants(int base, String className, PatchSpec spec) {
            next = base;
            int ownerClassName = utf8(className);
            int ownerClass = clazz(ownerClassName);
            int hookClassName = utf8(HOOK_CLASS);
            int hookClass = clazz(hookClassName);
            codeName = utf8(CODE);

            for (Target target : spec.targets) {
                target.renamedNameIndex = utf8(target.renamedName);
                int descriptor = utf8(target.descriptor);
                int nameAndType = nameAndType(target.renamedNameIndex, descriptor);
                target.renamedMethodRef = methodRef(ownerClass, nameAndType);
            }

            capturePositionsMethodRef = hookMethod(hookClass, CAPTURE_POSITIONS, CAPTURE_POSITIONS_DESCRIPTOR);
            afterEngineWrapMethodRef = hookMethod(hookClass, AFTER_ENGINE_WRAP, ENGINE_HOOK_DESCRIPTOR);
            afterEngineUnwrapMethodRef = hookMethod(hookClass, AFTER_ENGINE_UNWRAP, ENGINE_HOOK_DESCRIPTOR);
            afterSocketWriteMethodRef = hookMethod(hookClass, AFTER_SOCKET_WRITE, SOCKET_WRITE_DESCRIPTOR);
            afterSocketWriteByteMethodRef = hookMethod(hookClass, AFTER_SOCKET_WRITE_BYTE, SOCKET_WRITE_BYTE_DESCRIPTOR);
            afterSocketReadMethodRef = hookMethod(hookClass, AFTER_SOCKET_READ, SOCKET_READ_DESCRIPTOR);
            afterSocketReadByteMethodRef = hookMethod(hookClass, AFTER_SOCKET_READ_BYTE, SOCKET_READ_BYTE_DESCRIPTOR);
        }

        private int count() {
            return entries.size();
        }

        private void write(DataOutputStream out) throws IOException {
            for (ConstantWriter entry : entries) {
                entry.write(out);
            }
        }

        private int hookMethod(int hookClass, String name, String descriptor) {
            int nameIndex = utf8(name);
            int descriptorIndex = utf8(descriptor);
            return methodRef(hookClass, nameAndType(nameIndex, descriptorIndex));
        }

        private int utf8(String value) {
            int index = next++;
            entries.add(out -> writeUtf8(out, value));
            return index;
        }

        private int clazz(int nameIndex) {
            int index = next++;
            entries.add(out -> writeClass(out, nameIndex));
            return index;
        }

        private int nameAndType(int nameIndex, int descriptorIndex) {
            int index = next++;
            entries.add(out -> writeNameAndType(out, nameIndex, descriptorIndex));
            return index;
        }

        private int methodRef(int classIndex, int nameAndTypeIndex) {
            int index = next++;
            entries.add(out -> writeMethodRef(out, classIndex, nameAndTypeIndex));
            return index;
        }
    }

    private interface ConstantWriter {
        void write(DataOutputStream out) throws IOException;
    }

    private static final class ConstantPool {
        private final Entry[] entries;
        private final int endOffset;

        private ConstantPool(Entry[] entries, int endOffset) {
            this.entries = entries;
            this.endOffset = endOffset;
        }

        private String utf8(int index) {
            Entry entry = index > 0 && index < entries.length ? entries[index] : null;
            return entry != null ? entry.utf8 : null;
        }
    }

    private static final class Entry {
        private final int tag;
        private int a;
        private int b;
        private String utf8;

        private Entry(int tag) {
            this.tag = tag;
        }
    }

    private static final class MethodInfo {
        private final int startOffset;
        private final int endOffset;
        private final int accessFlags;
        private final int nameIndex;
        private final int descriptorIndex;
        private final int attributeCount;
        private final int attributesStart;
        private final List<AttributeInfo> attributes;
        private final String name;
        private final String descriptor;

        private MethodInfo(
                int startOffset,
                int endOffset,
                int accessFlags,
                int nameIndex,
                int descriptorIndex,
                int attributeCount,
                int attributesStart,
                List<AttributeInfo> attributes,
                String name,
                String descriptor) {
            this.startOffset = startOffset;
            this.endOffset = endOffset;
            this.accessFlags = accessFlags;
            this.nameIndex = nameIndex;
            this.descriptorIndex = descriptorIndex;
            this.attributeCount = attributeCount;
            this.attributesStart = attributesStart;
            this.attributes = attributes;
            this.name = name;
            this.descriptor = descriptor;
        }

        private static MethodInfo read(byte[] input, int offset, ConstantPool constantPool) {
            int start = offset;
            int accessFlags = u2(input, offset);
            int nameIndex = u2(input, offset + 2);
            int descriptorIndex = u2(input, offset + 4);
            int attributeCount = u2(input, offset + 6);
            offset += 8;
            int attributesStart = offset;
            List<AttributeInfo> attributes = new ArrayList<>();
            for (int i = 0; i < attributeCount; i++) {
                int attributeStart = offset;
                int attributeNameIndex = u2(input, offset);
                int attributeLength = u4(input, offset + 2);
                offset += 6 + attributeLength;
                attributes.add(new AttributeInfo(attributeStart, offset, constantPool.utf8(attributeNameIndex)));
            }
            return new MethodInfo(
                    start,
                    offset,
                    accessFlags,
                    nameIndex,
                    descriptorIndex,
                    attributeCount,
                    attributesStart,
                    attributes,
                    constantPool.utf8(nameIndex),
                    constantPool.utf8(descriptorIndex));
        }

        private void writeRenamed(DataOutputStream out, byte[] input, int renamedNameIndex)
                throws IOException {
            out.writeShort(accessFlags);
            out.writeShort(renamedNameIndex);
            out.writeShort(descriptorIndex);
            out.writeShort(attributeCount);
            out.write(input, attributesStart, endOffset - attributesStart);
        }

        private List<AttributeInfo> exceptionsAttributes() {
            List<AttributeInfo> exceptions = new ArrayList<>();
            for (AttributeInfo attribute : attributes) {
                if (EXCEPTIONS.equals(attribute.name)) {
                    exceptions.add(attribute);
                }
            }
            return exceptions;
        }
    }

    private static final class AttributeInfo {
        private final int startOffset;
        private final int endOffset;
        private final String name;

        private AttributeInfo(int startOffset, int endOffset, String name) {
            this.startOffset = startOffset;
            this.endOffset = endOffset;
            this.name = name;
        }
    }

    private static final class WrapperMethod {
        private final MethodInfo original;
        private final Target target;

        private WrapperMethod(MethodInfo original, Target target) {
            this.original = original;
            this.target = target;
        }

        private void write(DataOutputStream out, AddedConstants added, byte[] input) throws IOException {
            List<AttributeInfo> exceptions = original.exceptionsAttributes();
            out.writeShort(original.accessFlags);
            out.writeShort(original.nameIndex);
            out.writeShort(original.descriptorIndex);
            out.writeShort(1 + exceptions.size());
            writeCodeAttribute(out, added);
            for (AttributeInfo attribute : exceptions) {
                out.write(input, attribute.startOffset, attribute.endOffset - attribute.startOffset);
            }
        }

        private void writeCodeAttribute(DataOutputStream out, AddedConstants added) throws IOException {
            ByteArrayOutputStream code = new ByteArrayOutputStream();
            writeWrapperCode(code, added, target);

            ByteArrayOutputStream attribute = new ByteArrayOutputStream();
            DataOutputStream codeOut = new DataOutputStream(attribute);
            codeOut.writeShort(target.maxStack);
            codeOut.writeShort(target.maxLocals());
            codeOut.writeInt(code.size());
            codeOut.write(code.toByteArray());
            codeOut.writeShort(0);
            codeOut.writeShort(0);

            out.writeShort(added.codeName);
            out.writeInt(attribute.size());
            out.write(attribute.toByteArray());
        }
    }

}
