FROM ubuntu:24.04

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       python3 ca-certificates libxcb1 zlib1g libdbus-1-3 libxdo3 \
    && rm -rf /var/lib/apt/lists/* \
    && printf 'br:x:10001:10001:Assessment owner:/home/br:/bin/sh\nobserver:x:10002:10002:Assessment observer:/home/observer:/bin/sh\n' >> /etc/passwd \
    && printf 'br:x:10001:\nobserver:x:10002:\n' >> /etc/group \
    && mkdir -p /home/br /home/observer /probe \
    && chown 10001:10001 /home/br \
    && chown 10002:10002 /home/observer \
    && chmod 755 /home/br /home/observer

USER 10001:10001
WORKDIR /home/br
CMD ["sleep", "600"]
