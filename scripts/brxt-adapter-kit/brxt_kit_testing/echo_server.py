"""Loopback HTTPS echo servers for the kit's leak tests.

Each ``EchoServer`` listens on 127.0.0.1 (or ``localhost``) on its own port, so
every server is its own origin. All servers write into one shared ``EchoLog``,
which records every request in full: method, path, query, every header and the
body. The tests read the log to prove where each credential went.

The certificate is made fresh for each run by ``make_test_ca``: a throwaway
certificate authority and one server certificate for ``localhost``, 127.0.0.1
and ::1. Nothing is installed in any trust store; a test trusts the authority by
passing its file (``SSL_CERT_FILE`` for a subprocess, an ``ssl.SSLContext`` in
process).

Behaviour, selected by query parameters so it works under any route:

* ``__redirect_to=<url>`` with ``__redirect_code=<301|302|303|307|308>``:
  answer with that redirect.
* ``__bytes=<n>``: answer with ``n`` bytes of text.
* ``__gzip_bytes=<n>``: answer with ``n`` zero bytes, gzip compressed.
* ``__gzip_members=<n>``, with ``__member_bytes=<m>`` (default 1000): answer
  with ``n`` gzip members of ``m`` bytes each, labelled ``Content-Encoding:
  gzip``, which is what bgzip writes. ``gzip_member(k, m)`` is member ``k``'s
  content, so a test can check the order.
* ``__gzip_tail=<n>``: answer with the gzip stream of ``hi`` followed by ``n``
  zero bytes, streamed until the client stops reading.
* ``__bad_gzip=1``: answer with a body labelled gzip that is not gzip.
* ``__sleep=<seconds>``: wait before answering.
* ``__status=<code>``: answer with that status.
* ``__binary=1``: answer with binary content.
* anything else: answer 200 with a JSON echo of the request, headers included,
  so a test can also prove the kit removes secrets from what it returns.

Run it by hand with ``python echo_server.py`` to get three servers and their CA
path printed; requests are printed as they arrive.
"""

from __future__ import annotations

import datetime
import gzip
import ipaddress
import json
import ssl
import threading
import time
import urllib.parse
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any, Optional


def gzip_member(index: int, size: int) -> bytes:
    """The content of member ``index`` of a ``__gzip_members`` reply."""

    return (f"member-{index:06d};".encode() * (size // 14 + 1))[:size]


@dataclass
class RecordedRequest:
    server: str
    origin: str
    method: str
    path: str
    query: list[tuple[str, str]]
    headers: list[tuple[str, str]]
    body: bytes

    def header(self, name: str) -> Optional[str]:
        for key, value in self.headers:
            if key.lower() == name.lower():
                return value
        return None

    def raw_text(self) -> str:
        """Everything the request carried, as one string, for 'is this secret anywhere' checks."""

        parts = [self.method, self.path, urllib.parse.urlencode(self.query)]
        parts += [f"{k}: {v}" for k, v in self.headers]
        parts.append(self.body.decode("utf-8", errors="replace"))
        return "\n".join(parts)


@dataclass
class EchoLog:
    requests: list[RecordedRequest] = field(default_factory=list)
    lock: threading.Lock = field(default_factory=threading.Lock)

    def add(self, request: RecordedRequest) -> None:
        with self.lock:
            self.requests.append(request)

    def clear(self) -> None:
        with self.lock:
            self.requests.clear()

    def snapshot(self) -> list[RecordedRequest]:
        with self.lock:
            return list(self.requests)

    def at(self, server: str) -> list[RecordedRequest]:
        return [r for r in self.snapshot() if r.server == server]


@dataclass(frozen=True)
class TestCA:
    ca_file: Path
    cert_file: Path
    key_file: Path

    def client_context(self) -> ssl.SSLContext:
        return ssl.create_default_context(cafile=str(self.ca_file))

    def server_context(self) -> ssl.SSLContext:
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(str(self.cert_file), str(self.key_file))
        return context


def make_test_ca(directory: Path) -> TestCA:
    """Write a throwaway CA and a server certificate for loopback names into ``directory``."""

    from cryptography import x509
    from cryptography.hazmat.primitives import hashes, serialization
    from cryptography.hazmat.primitives.asymmetric import ec
    from cryptography.x509.oid import ExtendedKeyUsageOID, NameOID

    directory.mkdir(parents=True, exist_ok=True)
    now = datetime.datetime.now(datetime.timezone.utc)
    ca_key = ec.generate_private_key(ec.SECP256R1())
    ca_name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "brxt-adapter-kit test CA")])
    ca_cert = (
        x509.CertificateBuilder()
        .subject_name(ca_name)
        .issuer_name(ca_name)
        .public_key(ca_key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(now - datetime.timedelta(minutes=5))
        .not_valid_after(now + datetime.timedelta(days=2))
        .add_extension(x509.BasicConstraints(ca=True, path_length=0), critical=True)
        .add_extension(
            x509.KeyUsage(
                digital_signature=True,
                content_commitment=False,
                key_encipherment=False,
                data_encipherment=False,
                key_agreement=False,
                key_cert_sign=True,
                crl_sign=True,
                encipher_only=False,
                decipher_only=False,
            ),
            critical=True,
        )
        .add_extension(x509.SubjectKeyIdentifier.from_public_key(ca_key.public_key()), critical=False)
        .sign(ca_key, hashes.SHA256())
    )
    key = ec.generate_private_key(ec.SECP256R1())
    cert = (
        x509.CertificateBuilder()
        .subject_name(x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "localhost")]))
        .issuer_name(ca_name)
        .public_key(key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(now - datetime.timedelta(minutes=5))
        .not_valid_after(now + datetime.timedelta(days=2))
        .add_extension(
            x509.SubjectAlternativeName(
                [
                    x509.DNSName("localhost"),
                    x509.IPAddress(ipaddress.ip_address("127.0.0.1")),
                    x509.IPAddress(ipaddress.ip_address("::1")),
                ]
            ),
            critical=False,
        )
        .add_extension(x509.BasicConstraints(ca=False, path_length=None), critical=True)
        .add_extension(
            x509.KeyUsage(
                digital_signature=True,
                content_commitment=False,
                key_encipherment=True,
                data_encipherment=False,
                key_agreement=False,
                key_cert_sign=False,
                crl_sign=False,
                encipher_only=False,
                decipher_only=False,
            ),
            critical=True,
        )
        .add_extension(x509.ExtendedKeyUsage([ExtendedKeyUsageOID.SERVER_AUTH]), critical=False)
        .add_extension(x509.SubjectKeyIdentifier.from_public_key(key.public_key()), critical=False)
        .add_extension(
            x509.AuthorityKeyIdentifier.from_issuer_public_key(ca_key.public_key()),
            critical=False,
        )
        .sign(ca_key, hashes.SHA256())
    )
    ca_file = directory / "test-ca.pem"
    cert_file = directory / "server.pem"
    key_file = directory / "server-key.pem"
    ca_file.write_bytes(ca_cert.public_bytes(serialization.Encoding.PEM))
    cert_file.write_bytes(cert.public_bytes(serialization.Encoding.PEM))
    key_file.write_bytes(
        key.private_bytes(
            serialization.Encoding.PEM,
            serialization.PrivateFormat.PKCS8,
            serialization.NoEncryption(),
        )
    )
    return TestCA(ca_file, cert_file, key_file)


class _Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "brxt-echo/1"

    def log_message(self, format: str, *args: Any) -> None:  # noqa: A002
        if getattr(self.server, "verbose", False):
            super().log_message(format, *args)

    def _handle(self) -> None:
        length = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(length) if length else b""
        split = urllib.parse.urlsplit(self.path)
        query = urllib.parse.parse_qsl(split.query, keep_blank_values=True)
        echo: "EchoServer" = self.server.echo  # type: ignore[attr-defined]
        echo.log.add(
            RecordedRequest(
                server=echo.name,
                origin=echo.origin,
                method=self.command,
                path=split.path,
                query=query,
                headers=list(self.headers.items()),
                body=body,
            )
        )
        options = dict(query)
        if "__sleep" in options:
            time.sleep(float(options["__sleep"]))
        if "__redirect_to" in options:
            code = int(options.get("__redirect_code", "302"))
            self.send_response(code)
            self.send_header("Location", options["__redirect_to"])
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        status = int(options.get("__status", "200"))
        if "__bytes" in options:
            payload = b"a" * int(options["__bytes"])
            self._reply(status, "text/plain; charset=utf-8", payload)
            return
        if "__gzip_bytes" in options:
            payload = gzip.compress(b"\0" * int(options["__gzip_bytes"]))
            self._reply(status, "text/plain", payload, encoding="gzip")
            return
        if "__gzip_members" in options:
            size = int(options.get("__member_bytes", "1000"))
            payload = b"".join(gzip.compress(gzip_member(k, size)) for k in range(int(options["__gzip_members"])))
            self._reply(status, "text/plain", payload, encoding="gzip")
            return
        if "__gzip_tail" in options:
            self._stream_gzip_tail(status, int(options["__gzip_tail"]))
            return
        if "__bad_gzip" in options:
            bad = b"\x1f\x8b\x08\x00\x00\x00\x00\x00\x00\xff" + b"\x01\x05\x00\x00\x00hello" + b"\0" * 64
            self._reply(status, "text/plain", bad, encoding="gzip")
            return
        if "__binary" in options:
            self._reply(status, "application/octet-stream", bytes(range(256)))
            return
        payload = json.dumps(
            {
                "server": echo.name,
                "method": self.command,
                "path": split.path,
                "query": query,
                "headers": dict(self.headers.items()),
                "body": body.decode("utf-8", errors="replace"),
            }
        ).encode()
        self._reply(status, "application/json", payload)

    def _stream_gzip_tail(self, status: int, tail_bytes: int) -> None:
        self.send_response(status)
        self.send_header("Content-Type", "text/plain")
        self.send_header("Content-Encoding", "gzip")
        self.send_header("Connection", "close")
        self.end_headers()
        self.close_connection = True
        chunk = b"\0" * (1 << 20)
        try:
            self.wfile.write(gzip.compress(b"hi"))
            while tail_bytes > 0:
                self.wfile.write(chunk[:tail_bytes])
                tail_bytes -= len(chunk)
        except OSError:
            pass  # the client stopped reading, as it should

    def _reply(self, status: int, content_type: str, payload: bytes, encoding: Optional[str] = None) -> None:
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        if encoding:
            self.send_header("Content-Encoding", encoding)
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(payload)

    do_GET = do_POST = do_PUT = do_PATCH = do_DELETE = do_HEAD = do_OPTIONS = _handle


class _QuietServer(ThreadingHTTPServer):
    daemon_threads = True

    def handle_error(self, request: Any, client_address: Any) -> None:
        # A client that stops reading early (the kit does, past its size cap) resets
        # the connection; that is expected and not worth a traceback.
        if getattr(self, "verbose", False):
            super().handle_error(request, client_address)


class EchoServer:
    """One HTTPS echo server on its own loopback port, recording into a shared log."""

    def __init__(self, name: str, ca: TestCA, log: EchoLog, *, host: str = "127.0.0.1", verbose: bool = False) -> None:
        self.name = name
        self.host = host
        self.log = log
        bind = "127.0.0.1" if host in {"127.0.0.1", "localhost"} else host
        self._httpd = _QuietServer((bind, 0), _Handler)
        self._httpd.echo = self  # type: ignore[attr-defined]
        self._httpd.verbose = verbose  # type: ignore[attr-defined]
        context = ca.server_context()
        self._httpd.socket = context.wrap_socket(self._httpd.socket, server_side=True)
        self.port = self._httpd.server_address[1]
        self._thread = threading.Thread(target=self._httpd.serve_forever, name=f"echo-{name}", daemon=True)

    @property
    def origin(self) -> str:
        return f"https://{self.host}:{self.port}"

    def start(self) -> "EchoServer":
        self._thread.start()
        return self

    def stop(self) -> None:
        self._httpd.shutdown()
        self._httpd.server_close()
        self._thread.join(timeout=5)


class EchoFleet:
    """A set of echo servers sharing one log and one CA, stopped together."""

    def __init__(self, directory: Path, names: list[str], *, hosts: Optional[dict[str, str]] = None, verbose: bool = False) -> None:
        self.ca = make_test_ca(directory)
        self.log = EchoLog()
        hosts = hosts or {}
        self.servers = {
            name: EchoServer(name, self.ca, self.log, host=hosts.get(name, "127.0.0.1"), verbose=verbose).start()
            for name in names
        }

    def __getitem__(self, name: str) -> EchoServer:
        return self.servers[name]

    def stop(self) -> None:
        for server in self.servers.values():
            server.stop()

    def __enter__(self) -> "EchoFleet":
        return self

    def __exit__(self, *exc: Any) -> None:
        self.stop()


def main() -> None:
    import tempfile

    directory = Path(tempfile.mkdtemp(prefix="brxt-echo-"))
    fleet = EchoFleet(directory, ["alpha", "beta", "outsider"], hosts={"beta": "localhost"}, verbose=True)
    print(f"CA file: {fleet.ca.ca_file}")
    for server in fleet.servers.values():
        print(f"{server.name}: {server.origin}")
    try:
        while True:
            time.sleep(3600)
    except KeyboardInterrupt:
        fleet.stop()


if __name__ == "__main__":
    main()
