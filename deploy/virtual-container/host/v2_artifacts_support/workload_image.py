"""Build a Firecracker workload image with the real xiaoO preinstalled."""

from __future__ import annotations

import copy
import hashlib
import io
import json
import os
import tarfile
import tempfile
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from typing import Any, BinaryIO

from tests.v2.common.kata_runtime.image import firecracker_workload_reference


XIAOO_CONTAINER_PATH = "/opt/actrail-execution/xiaoo-real"
_XIAOO_LAYER_PATH = XIAOO_CONTAINER_PATH.removeprefix("/")
_LAYER_MEDIA_TYPE = "application/vnd.oci.image.layer.v1.tar"
_CONFIG_MEDIA_TYPE = "application/vnd.oci.image.config.v1+json"
_MANIFEST_MEDIA_TYPE = "application/vnd.oci.image.manifest.v1+json"
_INDEX_MEDIA_TYPE = "application/vnd.oci.image.index.v1+json"


@dataclass(frozen=True)
class PreparedWorkloadImage:
    reference: str
    archive: Path
    archive_sha256: str
    xiaoo_path: str
    xiaoo_sha256: str


@dataclass(frozen=True)
class _PreparedOciMetadata:
    index_payload: bytes
    manifest_path: str
    manifest_payload: bytes
    original_manifest_path: str


class _HashingReader:
    def __init__(self, source: BinaryIO) -> None:
        self._source = source
        self._digest = hashlib.sha256()
        self.bytes_read = 0

    def read(self, size: int = -1) -> bytes:
        block = self._source.read(size)
        self._digest.update(block)
        self.bytes_read += len(block)
        return block

    def hexdigest(self) -> str:
        return self._digest.hexdigest()


def prepare_firecracker_workload_image(
    source_archive: Path,
    xiaoo: Path,
    output_archive: Path,
    cache_key: str,
    *,
    source_reference: str,
    source_archive_sha256: str,
    xiaoo_sha256: str,
    expected_architecture: str,
) -> PreparedWorkloadImage:
    """Append xiaoO as an immutable image layer and publish a unique tag."""
    reference = firecracker_workload_reference(cache_key)
    output_archive.parent.mkdir(parents=True, exist_ok=True)
    layer_path = _build_xiaoo_layer(
        xiaoo,
        output_archive.parent,
        xiaoo_sha256,
    )
    temporary_output: Path | None = None
    try:
        layer_sha256 = _sha256_file(layer_path)
        layer_size = layer_path.stat().st_size
        with tempfile.TemporaryFile(
            mode="w+b",
            dir=output_archive.parent,
        ) as source_snapshot:
            _copy_verified_file(
                source_archive,
                source_snapshot,
                source_archive_sha256,
                "source archive",
            )
            with tarfile.open(fileobj=source_snapshot, mode="r:*") as source:
                members = _validated_members(source)
                manifest = _docker_manifest(source, members)
                _validate_base_reference(manifest, source_reference)
                original_config_path = _required_string(manifest, "Config")
                config = _json_object(
                    _member_bytes(source, members, original_config_path),
                    "Docker image config",
                )
                config_os, config_architecture = _validate_base_platform(
                    config,
                    expected_architecture,
                )
                config_diff_ids = _rootfs_diff_ids(config)
                _validate_docker_layers(
                    source,
                    members,
                    manifest,
                    config_diff_ids,
                )
                config["rootfs"] = _updated_rootfs(config, layer_sha256)
                config["history"] = _updated_history(config)
                config_payload = _canonical_json(config)
                config_sha256 = hashlib.sha256(config_payload).hexdigest()
                config_path = f"blobs/sha256/{config_sha256}"
                layer_blob_path = f"blobs/sha256/{layer_sha256}"
                updated_manifest = _updated_manifest(
                    manifest,
                    reference=reference,
                    config_path=config_path,
                    layer_path=layer_blob_path,
                    layer_sha256=layer_sha256,
                    layer_size=layer_size,
                )
                oci_metadata = _updated_oci_metadata(
                    source,
                    members,
                    docker_manifest=manifest,
                    source_reference=source_reference,
                    config_os=config_os,
                    config_architecture=config_architecture,
                    config_diff_ids=config_diff_ids,
                    reference=reference,
                    original_config_path=original_config_path,
                    config_path=config_path,
                    config_size=len(config_payload),
                    layer_sha256=layer_sha256,
                    layer_size=layer_size,
                )
                generated_blob_paths = {
                    config_path,
                    layer_blob_path,
                }
                if oci_metadata is not None:
                    generated_blob_paths.add(oci_metadata.manifest_path)
                generated_collisions = generated_blob_paths.intersection(members)
                if generated_collisions:
                    raise ValueError(
                        "Docker workload image archive already contains generated "
                        "blob(s): " + ", ".join(sorted(generated_collisions))
                    )
                repository, tag = reference.rsplit(":", 1)
                repositories_payload = _canonical_json(
                    {repository: {tag: layer_sha256}}
                )
                manifest_payload = _canonical_json([updated_manifest])
                descriptor, temporary_name = tempfile.mkstemp(
                    prefix=f".{output_archive.name}.",
                    dir=output_archive.parent,
                )
                temporary_output = Path(temporary_name)
                with os.fdopen(descriptor, "w+b") as raw_output:
                    with tarfile.open(
                        fileobj=raw_output,
                        mode="w",
                    ) as destination:
                        omitted = {"manifest.json", "repositories"}
                        if oci_metadata is not None:
                            omitted.update(
                                {
                                    "index.json",
                                    oci_metadata.original_manifest_path,
                                }
                            )
                        _copy_source_members(
                            source,
                            destination,
                            members,
                            omitted=omitted,
                        )
                        _add_file(destination, config_path, config_payload)
                        with layer_path.open("rb") as layer_source:
                            verified_layer = _HashingReader(layer_source)
                            _add_stream(
                                destination,
                                layer_blob_path,
                                verified_layer,
                                layer_size,
                            )
                            _require_stream_digest(
                                verified_layer,
                                layer_size,
                                layer_sha256,
                                "generated xiaoO layer",
                            )
                        if oci_metadata is not None:
                            _add_file(
                                destination,
                                oci_metadata.manifest_path,
                                oci_metadata.manifest_payload,
                            )
                            _add_file(
                                destination,
                                "index.json",
                                oci_metadata.index_payload,
                            )
                        _add_file(
                            destination,
                            "manifest.json",
                            manifest_payload,
                        )
                        _add_file(
                            destination,
                            "repositories",
                            repositories_payload,
                        )
                    raw_output.flush()
                    os.fsync(raw_output.fileno())
                with tarfile.open(temporary_output, mode="r:") as prepared:
                    _validated_members(prepared)
                os.replace(temporary_output, output_archive)
                temporary_output = None
    except (KeyError, json.JSONDecodeError, tarfile.TarError) as error:
        raise ValueError(
            f"invalid Docker workload image archive {source_archive}: {error}"
        ) from error
    finally:
        layer_path.unlink(missing_ok=True)
        if temporary_output is not None:
            temporary_output.unlink(missing_ok=True)

    return PreparedWorkloadImage(
        reference=reference,
        archive=output_archive,
        archive_sha256=_sha256_file(output_archive),
        xiaoo_path=XIAOO_CONTAINER_PATH,
        xiaoo_sha256=xiaoo_sha256,
    )


def _build_xiaoo_layer(
    xiaoo: Path,
    directory: Path,
    expected_sha256: str,
) -> Path:
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=".firecracker-xiaoo-layer.",
        dir=directory,
    )
    layer_path = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "w+b") as raw_layer:
            with tarfile.open(
                fileobj=raw_layer,
                mode="w",
                format=tarfile.PAX_FORMAT,
            ) as layer:
                directory_info = tarfile.TarInfo(
                    str(PurePosixPath(_XIAOO_LAYER_PATH).parent) + "/"
                )
                directory_info.type = tarfile.DIRTYPE
                directory_info.mode = 0o755
                directory_info.uid = 0
                directory_info.gid = 0
                directory_info.mtime = 0
                layer.addfile(directory_info)
                file_info = tarfile.TarInfo(_XIAOO_LAYER_PATH)
                file_info.mode = 0o755
                file_info.uid = 0
                file_info.gid = 0
                file_info.mtime = 0
                with xiaoo.open("rb") as source:
                    file_info.size = os.fstat(source.fileno()).st_size
                    verified_source = _HashingReader(source)
                    layer.addfile(file_info, verified_source)
                    _require_stream_digest(
                        verified_source,
                        file_info.size,
                        expected_sha256,
                        "xiaoO",
                    )
            raw_layer.flush()
            os.fsync(raw_layer.fileno())
        return layer_path
    except BaseException:
        layer_path.unlink(missing_ok=True)
        raise


def _validated_members(archive: tarfile.TarFile) -> dict[str, tarfile.TarInfo]:
    members: dict[str, tarfile.TarInfo] = {}
    for member in archive.getmembers():
        path = PurePosixPath(member.name)
        name = path.as_posix()
        if (
            path.is_absolute()
            or not path.parts
            or ".." in path.parts
            or name in members
        ):
            raise ValueError(
                "unsafe or duplicate Docker archive member: " + member.name
            )
        if not (member.isfile() or member.isdir()):
            raise ValueError(f"unsupported Docker archive member: {member.name}")
        members[name] = member
    return members


def _docker_manifest(
    archive: tarfile.TarFile,
    members: dict[str, tarfile.TarInfo],
) -> dict[str, Any]:
    document = json.loads(_member_bytes(archive, members, "manifest.json"))
    if not isinstance(document, list) or len(document) != 1:
        raise ValueError("Docker archive manifest must contain exactly one image")
    if not isinstance(document[0], dict):
        raise ValueError("Docker archive manifest entry must be an object")
    return copy.deepcopy(document[0])


def _validate_base_reference(
    manifest: dict[str, Any],
    expected: str,
) -> None:
    expected_canonical = _canonical_reference(expected)
    repo_tags = manifest.get("RepoTags")
    if not isinstance(repo_tags, list) or not repo_tags or not all(
        isinstance(value, str) and value for value in repo_tags
    ):
        raise ValueError(
            "Docker base image reference list must contain strings"
        )
    if any(
        _canonical_reference(value) != expected_canonical
        for value in repo_tags
    ):
        raise ValueError(
            "Docker base image reference does not match the configured "
            f"workload image: configured={expected} archive={repo_tags}"
        )


def _validate_base_platform(
    config: dict[str, Any],
    expected_architecture: str,
) -> tuple[str, str]:
    image_os = config.get("os")
    architecture = config.get("architecture")
    if image_os != "linux" or architecture != expected_architecture:
        raise ValueError(
            "Docker base image platform does not match the Firecracker host: "
            f"expected=linux/{expected_architecture} "
            f"archive={image_os}/{architecture}"
        )
    return image_os, architecture


def _canonical_reference(reference: str) -> str:
    if (
        not reference
        or reference != reference.strip()
        or "@" in reference
        or ":" not in reference.rsplit("/", 1)[-1]
    ):
        raise ValueError(
            "Docker base image reference must contain an explicit tag"
        )
    components = reference.split("/")
    if len(components) == 1:
        return "docker.io/library/" + reference
    first = components[0]
    if "." not in first and ":" not in first and first != "localhost":
        return "docker.io/" + reference
    if first == "docker.io" and len(components) == 2:
        return "docker.io/library/" + components[1]
    return reference


def _rootfs_diff_ids(config: dict[str, Any]) -> list[str]:
    rootfs = config.get("rootfs")
    if not isinstance(rootfs, dict) or rootfs.get("type") != "layers":
        raise ValueError("Docker image config rootfs must use layers")
    diff_ids = rootfs.get("diff_ids")
    if not isinstance(diff_ids, list) or not all(
        isinstance(value, str) for value in diff_ids
    ):
        raise ValueError(
            "Docker image config rootfs.diff_ids must be a string list"
        )
    for digest in diff_ids:
        _sha256_blob_path(digest)
    return list(diff_ids)


def _validate_docker_layers(
    archive: tarfile.TarFile,
    members: dict[str, tarfile.TarInfo],
    manifest: dict[str, Any],
    diff_ids: list[str],
) -> None:
    layers = manifest.get("Layers")
    if not isinstance(layers, list) or not all(
        isinstance(value, str) for value in layers
    ):
        raise ValueError("Docker archive manifest Layers must be a string list")
    if len(layers) != len(diff_ids):
        raise ValueError(
            "Docker image rootfs.diff_ids count does not match its layers"
        )
    layer_sources = manifest.get("LayerSources")
    if not isinstance(layer_sources, dict):
        raise ValueError("Docker archive LayerSources must be an object")
    for position, (layer_path, diff_id) in enumerate(
        zip(layers, diff_ids, strict=True),
        start=1,
    ):
        expected_path = _sha256_blob_path(diff_id)
        if layer_path != expected_path:
            raise ValueError(
                f"Docker image layer {position} path does not match its diffID"
            )
        descriptor = layer_sources.get(diff_id)
        if not isinstance(descriptor, dict):
            raise ValueError(
                f"Docker image layer {position} source descriptor is missing"
            )
        if descriptor.get("mediaType") != _LAYER_MEDIA_TYPE:
            raise ValueError(
                f"Docker image layer {position} must be an uncompressed OCI tar"
            )
        _verify_member_descriptor(
            archive,
            members,
            descriptor,
            layer_path,
            f"Docker image layer {position}",
        )


def _updated_rootfs(config: dict[str, Any], layer_sha256: str) -> dict[str, Any]:
    rootfs = config["rootfs"]
    diff_ids = _rootfs_diff_ids(config)
    updated = copy.deepcopy(rootfs)
    updated["diff_ids"] = [*diff_ids, f"sha256:{layer_sha256}"]
    return updated


def _updated_history(config: dict[str, Any]) -> list[Any]:
    history = config.get("history", [])
    if not isinstance(history, list):
        raise ValueError("Docker image config history must be a list")
    return [
        *copy.deepcopy(history),
        {
            "created": "1970-01-01T00:00:00Z",
            "created_by": "AcTrail Firecracker artifact: preinstall xiaoO",
        },
    ]


def _updated_manifest(
    manifest: dict[str, Any],
    *,
    reference: str,
    config_path: str,
    layer_path: str,
    layer_sha256: str,
    layer_size: int,
) -> dict[str, Any]:
    layers = manifest.get("Layers")
    if not isinstance(layers, list) or not all(
        isinstance(value, str) for value in layers
    ):
        raise ValueError("Docker archive manifest Layers must be a string list")
    layer_sources = manifest.get("LayerSources", {})
    if not isinstance(layer_sources, dict):
        raise ValueError("Docker archive LayerSources must be an object")
    updated = copy.deepcopy(manifest)
    updated["Config"] = config_path
    updated["RepoTags"] = [reference]
    updated["Layers"] = [*layers, layer_path]
    updated_sources = copy.deepcopy(layer_sources)
    updated_sources[f"sha256:{layer_sha256}"] = {
        "mediaType": _LAYER_MEDIA_TYPE,
        "size": layer_size,
        "digest": f"sha256:{layer_sha256}",
    }
    updated["LayerSources"] = updated_sources
    return updated


def _updated_oci_metadata(
    archive: tarfile.TarFile,
    members: dict[str, tarfile.TarInfo],
    *,
    docker_manifest: dict[str, Any],
    source_reference: str,
    config_os: str,
    config_architecture: str,
    config_diff_ids: list[str],
    reference: str,
    original_config_path: str,
    config_path: str,
    config_size: int,
    layer_sha256: str,
    layer_size: int,
) -> _PreparedOciMetadata | None:
    has_index = "index.json" in members
    has_layout = "oci-layout" in members
    if not has_index and not has_layout:
        return None
    if not has_index or not has_layout:
        raise ValueError(
            "Docker workload image archive has an incomplete OCI layout"
        )
    layout = _json_object(
        _member_bytes(archive, members, "oci-layout"),
        "OCI image layout",
    )
    if layout.get("imageLayoutVersion") != "1.0.0":
        raise ValueError("OCI image layout version must be 1.0.0")

    index = _json_object(
        _member_bytes(archive, members, "index.json"),
        "OCI image index",
    )
    if (
        index.get("schemaVersion") != 2
        or index.get("mediaType") != _INDEX_MEDIA_TYPE
    ):
        raise ValueError("OCI image index schema or media type is unsupported")
    descriptors = index.get("manifests")
    if not isinstance(descriptors, list) or len(descriptors) != 1:
        raise ValueError("OCI image index must contain exactly one manifest")
    descriptor = descriptors[0]
    if not isinstance(descriptor, dict):
        raise ValueError("OCI image index manifest descriptor must be an object")
    if descriptor.get("mediaType") != _MANIFEST_MEDIA_TYPE:
        raise ValueError("OCI image manifest descriptor media type is unsupported")
    annotations = descriptor.get("annotations")
    if not isinstance(annotations, dict):
        raise ValueError("OCI image index annotations must be an object")
    expected_source_reference = _canonical_reference(source_reference)
    oci_source_reference = annotations.get("io.containerd.image.name")
    if oci_source_reference != expected_source_reference:
        raise ValueError(
            "OCI base image reference does not match the configured workload "
            f"image: configured={source_reference} "
            f"archive={oci_source_reference}"
        )
    expected_tag = expected_source_reference.rsplit(":", 1)[1]
    if annotations.get("org.opencontainers.image.ref.name") != expected_tag:
        raise ValueError(
            "OCI base image tag annotation does not match the configured "
            f"workload image: expected={expected_tag} "
            f"archive={annotations.get('org.opencontainers.image.ref.name')}"
        )
    platform = descriptor.get("platform")
    if not isinstance(platform, dict) or (
        platform.get("os") != config_os
        or platform.get("architecture") != config_architecture
    ):
        raise ValueError(
            "OCI base image platform does not match its Docker config: "
            f"config={config_os}/{config_architecture} "
            f"descriptor={platform}"
        )
    original_manifest_path = _sha256_blob_path(
        _required_string(descriptor, "digest")
    )
    original_manifest_payload = _member_bytes(
        archive,
        members,
        original_manifest_path,
    )
    _verify_descriptor_payload(
        descriptor,
        original_manifest_payload,
        "OCI image manifest",
    )
    oci_manifest = _json_object(
        original_manifest_payload,
        "OCI image manifest",
    )
    if (
        oci_manifest.get("schemaVersion") != 2
        or oci_manifest.get("mediaType") != _MANIFEST_MEDIA_TYPE
    ):
        raise ValueError("OCI image manifest schema or media type is unsupported")
    config_descriptor = oci_manifest.get("config")
    if not isinstance(config_descriptor, dict):
        raise ValueError("OCI image config descriptor must be an object")
    if config_descriptor.get("mediaType") != _CONFIG_MEDIA_TYPE:
        raise ValueError("OCI image config descriptor media type is unsupported")
    config_platform = config_descriptor.get("platform")
    if not isinstance(config_platform, dict) or (
        config_platform.get("os") != config_os
        or config_platform.get("architecture") != config_architecture
    ):
        raise ValueError(
            "OCI image config descriptor platform does not match its config: "
            f"config={config_os}/{config_architecture} "
            f"descriptor={config_platform}"
        )
    if _sha256_blob_path(
        _required_string(config_descriptor, "digest")
    ) != original_config_path:
        raise ValueError(
            "Docker and OCI manifests do not reference the same image config"
        )
    _verify_member_descriptor(
        archive,
        members,
        config_descriptor,
        original_config_path,
        "OCI image config",
    )
    layer_descriptors = oci_manifest.get("layers")
    if not isinstance(layer_descriptors, list) or not all(
        isinstance(value, dict) for value in layer_descriptors
    ):
        raise ValueError("OCI image manifest layers must be an object list")
    if len(layer_descriptors) != len(config_diff_ids):
        raise ValueError(
            "OCI image layer count does not match config rootfs.diff_ids"
        )
    for position, (layer_descriptor, diff_id) in enumerate(
        zip(layer_descriptors, config_diff_ids, strict=True),
        start=1,
    ):
        if (
            layer_descriptor.get("mediaType") != _LAYER_MEDIA_TYPE
            or layer_descriptor.get("digest") != diff_id
        ):
            raise ValueError(
                f"OCI image layer {position} must be an uncompressed diffID tar"
            )
    docker_layers = docker_manifest.get("Layers")
    if not isinstance(docker_layers, list) or [
        _sha256_blob_path(_required_string(value, "digest"))
        for value in layer_descriptors
    ] != docker_layers:
        raise ValueError(
            "Docker and OCI manifests do not reference the same image layers"
        )
    for position, (layer_descriptor, docker_layer) in enumerate(
        zip(layer_descriptors, docker_layers, strict=True),
        start=1,
    ):
        _verify_member_descriptor(
            archive,
            members,
            layer_descriptor,
            docker_layer,
            f"OCI image layer {position}",
        )

    updated_config_descriptor = copy.deepcopy(config_descriptor)
    updated_config_descriptor["digest"] = (
        "sha256:" + PurePosixPath(config_path).name
    )
    updated_config_descriptor["size"] = config_size
    updated_oci_manifest = copy.deepcopy(oci_manifest)
    updated_oci_manifest["config"] = updated_config_descriptor
    updated_oci_manifest["layers"] = [
        *copy.deepcopy(layer_descriptors),
        {
            "mediaType": _LAYER_MEDIA_TYPE,
            "digest": f"sha256:{layer_sha256}",
            "size": layer_size,
        },
    ]
    manifest_payload = _canonical_json(updated_oci_manifest)
    manifest_sha256 = hashlib.sha256(manifest_payload).hexdigest()
    manifest_path = f"blobs/sha256/{manifest_sha256}"

    updated_descriptor = copy.deepcopy(descriptor)
    updated_descriptor["digest"] = f"sha256:{manifest_sha256}"
    updated_descriptor["size"] = len(manifest_payload)
    updated_annotations = copy.deepcopy(annotations)
    updated_annotations["io.containerd.image.name"] = reference
    updated_annotations["org.opencontainers.image.ref.name"] = reference.rsplit(
        ":", 1
    )[1]
    updated_descriptor["annotations"] = updated_annotations
    updated_index = copy.deepcopy(index)
    updated_index["manifests"] = [updated_descriptor]
    return _PreparedOciMetadata(
        index_payload=_canonical_json(updated_index),
        manifest_path=manifest_path,
        manifest_payload=manifest_payload,
        original_manifest_path=original_manifest_path,
    )


def _sha256_blob_path(digest: str) -> str:
    prefix = "sha256:"
    value = digest.removeprefix(prefix)
    if not digest.startswith(prefix) or len(value) != 64:
        raise ValueError(f"OCI descriptor digest is not SHA-256: {digest}")
    try:
        parsed = int(value, 16)
    except ValueError as error:
        raise ValueError(
            f"OCI descriptor digest is not SHA-256: {digest}"
        ) from error
    if value != f"{parsed:064x}":
        raise ValueError(f"OCI descriptor digest is not canonical: {digest}")
    return f"blobs/sha256/{value}"


def _verify_descriptor_payload(
    descriptor: dict[str, Any],
    payload: bytes,
    label: str,
) -> None:
    digest = _required_string(descriptor, "digest")
    expected_path = _sha256_blob_path(digest)
    actual_digest = hashlib.sha256(payload).hexdigest()
    if expected_path != f"blobs/sha256/{actual_digest}":
        raise ValueError(f"{label} digest does not match its descriptor")
    size = descriptor.get("size")
    if (
        not isinstance(size, int)
        or isinstance(size, bool)
        or size < 0
        or size != len(payload)
    ):
        raise ValueError(f"{label} size does not match its descriptor")


def _verify_member_descriptor(
    archive: tarfile.TarFile,
    members: dict[str, tarfile.TarInfo],
    descriptor: dict[str, Any],
    member_path: str,
    label: str,
) -> None:
    if _sha256_blob_path(
        _required_string(descriptor, "digest")
    ) != member_path:
        raise ValueError(f"{label} path does not match its descriptor")
    member = members.get(member_path)
    if member is None or not member.isfile():
        raise ValueError(f"{label} blob is missing: {member_path}")
    size = descriptor.get("size")
    if (
        not isinstance(size, int)
        or isinstance(size, bool)
        or size < 0
        or size != member.size
    ):
        raise ValueError(f"{label} size does not match its descriptor")
    source = archive.extractfile(member)
    if source is None:
        raise ValueError(f"{label} blob has no payload: {member_path}")
    digest = hashlib.sha256()
    while block := source.read(1024 * 1024):
        digest.update(block)
    if member_path != f"blobs/sha256/{digest.hexdigest()}":
        raise ValueError(f"{label} digest does not match its descriptor")


def _copy_source_members(
    source: tarfile.TarFile,
    destination: tarfile.TarFile,
    members: dict[str, tarfile.TarInfo],
    *,
    omitted: set[str],
) -> None:
    for name, member in members.items():
        if name in omitted:
            continue
        payload = source.extractfile(member) if member.isfile() else None
        copied = copy.copy(member)
        copied.name = name
        destination.addfile(copied, payload)


def _member_bytes(
    archive: tarfile.TarFile,
    members: dict[str, tarfile.TarInfo],
    name: str,
) -> bytes:
    member = members.get(name)
    if member is None or not member.isfile():
        raise ValueError(f"Docker archive member is missing: {name}")
    source = archive.extractfile(member)
    if source is None:
        raise ValueError(f"Docker archive member has no payload: {name}")
    return source.read()


def _json_object(payload: bytes, label: str) -> dict[str, Any]:
    document = json.loads(payload)
    if not isinstance(document, dict):
        raise ValueError(f"{label} must be an object")
    return document


def _required_string(document: dict[str, Any], field: str) -> str:
    value = document.get(field)
    if not isinstance(value, str) or not value:
        raise ValueError(f"Docker manifest {field} must be a non-empty string")
    return value


def _canonical_json(document: Any) -> bytes:
    return json.dumps(
        document,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")


def _add_file(archive: tarfile.TarFile, name: str, payload: bytes) -> None:
    _add_stream(archive, name, io.BytesIO(payload), len(payload))


def _add_stream(
    archive: tarfile.TarFile,
    name: str,
    source: BinaryIO,
    size: int,
) -> None:
    info = tarfile.TarInfo(name)
    info.size = size
    info.mode = 0o644
    info.uid = 0
    info.gid = 0
    info.mtime = 0
    archive.addfile(info, source)


def _copy_verified_file(
    source_path: Path,
    destination: BinaryIO,
    expected_sha256: str,
    label: str,
) -> None:
    _require_sha256(expected_sha256, label)
    digest = hashlib.sha256()
    with source_path.open("rb") as source:
        while block := source.read(1024 * 1024):
            digest.update(block)
            destination.write(block)
    actual_sha256 = digest.hexdigest()
    if actual_sha256 != expected_sha256:
        raise ValueError(
            f"{label} digest mismatch: expected={expected_sha256} "
            f"actual={actual_sha256}"
        )
    destination.flush()
    os.fsync(destination.fileno())
    destination.seek(0)


def _require_stream_digest(
    source: _HashingReader,
    expected_size: int,
    expected_sha256: str,
    label: str,
) -> None:
    _require_sha256(expected_sha256, label)
    if source.bytes_read != expected_size:
        raise ValueError(
            f"{label} size changed while reading: "
            f"expected={expected_size} actual={source.bytes_read}"
        )
    actual_sha256 = source.hexdigest()
    if actual_sha256 != expected_sha256:
        raise ValueError(
            f"{label} digest mismatch: expected={expected_sha256} "
            f"actual={actual_sha256}"
        )


def _require_sha256(value: str, label: str) -> None:
    if len(value) != 64:
        raise ValueError(
            f"{label} digest must be 64 lowercase hexadecimal digits"
        )
    try:
        parsed = int(value, 16)
    except ValueError as error:
        raise ValueError(
            f"{label} digest must be 64 lowercase hexadecimal digits"
        ) from error
    if value != f"{parsed:064x}":
        raise ValueError(
            f"{label} digest must be 64 lowercase hexadecimal digits"
        )


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while block := source.read(1024 * 1024):
            digest.update(block)
    return digest.hexdigest()
