# Wrangler config — `npx wrangler deploy worker.ts`.
#
# Pre-reqs (one-time):
#   1. `wrangler r2 bucket create <bucket>`
#   2. (optional) `wrangler kv:namespace create RATELIMIT`
#   3. (optional) `wrangler secret put AUTH_TOKEN`
#
# Run `tynd updater publish <ver> --target r2 --bucket <bucket> --endpoint
# https://<account-id>.r2.cloudflarestorage.com` to populate the bucket.

name = "tynd-update-server"
main = "worker.ts"
compatibility_date = "${compatibilityDate}"

# R2 bucket holding the published manifest + binaries + patches.
# Replace `tynd-updates` with the bucket name you used in `tynd updater publish --bucket`.
[[r2_buckets]]
binding = "BUCKET"
bucket_name = "tynd-updates"

# Optional: KV namespace for per-IP rate limiting. Comment-in the block
# after creating the namespace and pasting the printed `id` here.
# [[kv_namespaces]]
# binding = "RATELIMIT"
# id = "<paste-from-wrangler-kv-namespace-create>"

# Optional: rate limit cap per IP / minute. Override at runtime via
# `wrangler secret put RATE_LIMIT_PER_MIN`.
# [vars]
# RATE_LIMIT_PER_MIN = "60"
