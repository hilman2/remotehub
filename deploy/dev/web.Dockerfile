# Node container of the development environment (deploy/compose.dev.yml) for
# the SvelteKit frontend in web/. Vite polls (VITE_POLLING=1) because Windows
# does not pass file events into containers.
# Keep in sync with scripts/ci/tools.Dockerfile and web/package.json (packageManager).
FROM node:24.21.0-trixie-slim
RUN npm install -g pnpm@12.6.0 && pnpm --version
ENV TZ=Europe/Berlin
WORKDIR /work/web
