FROM python:3.12-slim@sha256:a0f57064cbdcd6285fa45f51237ab0b35ea8ebaed06a1f5973e89e19dfb86704

RUN apt-get update \
    && apt-get install -y --no-install-recommends libxcb1 \
    && rm -rf /var/lib/apt/lists/*
COPY biorouter-cli_1.93.4_amd64.deb /tmp/biorouter.deb
RUN echo '8b689a2006f1b99f44dec9a48a03b8bb67759dd7f11cdc288042a8a985abff65  /tmp/biorouter.deb' | sha256sum -c - \
    && dpkg-deb -x /tmp/biorouter.deb /opt/release \
    && mkdir -p /opt/biorouter /home/br /home/attacker /assessment/fixtures /assessment/harness /evidence \
    && cp /opt/release/usr/bin/biorouter /opt/release/usr/bin/biorouterd /opt/biorouter/ \
    && rm /tmp/biorouter.deb \
    && printf 'br:x:10001:10001:Assessment owner:/home/br:/bin/sh\nattacker:x:10002:10002:Assessment outsider:/home/attacker:/bin/sh\n' >> /etc/passwd \
    && printf 'br:x:10001:\nattacker:x:10002:\n' >> /etc/group \
    && chown 10001:10001 /home/br \
    && chown 10002:10002 /home/attacker \
    && chmod 755 /home/br /home/attacker /opt/biorouter/*
ENV PATH=/opt/biorouter:/usr/local/bin:/usr/bin:/bin
USER 10001:10001
WORKDIR /home/br
CMD ["sleep", "infinity"]
