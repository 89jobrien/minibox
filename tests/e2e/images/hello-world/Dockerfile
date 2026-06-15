# e2e test image: hello-world
# A minimal FROM scratch image that copies a static greeting file.
# Exercises: FROM scratch (no base pull), COPY, ENV, CMD.
#
# FROM scratch produces an empty rootfs layer stack — this is the
# simplest possible Dockerfile the minibox builder can process.
FROM scratch
COPY hello.txt /hello.txt
ENV GREETING=hello
CMD ["/bin/sh", "-c", "cat /hello.txt"]
