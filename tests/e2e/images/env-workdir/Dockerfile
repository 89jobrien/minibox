# e2e test image: env-workdir
# Exercises: FROM with implicit latest tag, ENV multi-pair form,
# WORKDIR, ARG, RUN shell form, CMD shell form.
#
# This fixture exercises the more complex instruction variants
# in minibox_core::image::dockerfile::parse.
FROM alpine
ARG BUILD_DATE=unknown
ENV APP=env-workdir VERSION=1.0 BUILD_DATE=${BUILD_DATE}
WORKDIR /app
RUN echo "build date: ${BUILD_DATE}" > /app/build-info.txt
CMD ["/bin/sh", "-c", "cat /app/build-info.txt"]
