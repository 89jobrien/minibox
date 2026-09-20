# e2e test image: alpine-echo
# A minimal image based on alpine that copies a shell script
# into the image and sets it as the default command.
#
# This Dockerfile exercises: FROM with tag, COPY, CMD.
# The format is standard Dockerfile syntax as parsed by
# minibox_core::image::dockerfile::parse.
FROM alpine:3.21
COPY echo.sh /usr/local/bin/echo.sh
ENV APP_NAME=alpine-echo
CMD ["/bin/sh", "/usr/local/bin/echo.sh"]
