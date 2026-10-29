FROM alpine:3.23@sha256:ff7f0c727dd796aa36787bfcd9a64bf35a6ce72a73451b3359fde849b6703d63
LABEL io.factory.environment.image="1"
RUN addgroup -g 10000 factory \
    && adduser -D -u 10000 -G factory -h /home/factory factory \
    && mkdir -p /home/factory/Documents \
    && chown -R 10000:10000 /home/factory
ENV HOME=/home/factory
USER 10000:10000
WORKDIR /home/factory/Documents
ENTRYPOINT ["/bin/sleep"]
CMD ["infinity"]
