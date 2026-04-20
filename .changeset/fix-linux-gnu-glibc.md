---
"@astrojs/compiler-rs": patch
---

Fixed linux-gnu binaries requiring glibc 2.35+, which broke on Vercel, Amazon Linux 2023, and other environments with older glibc. Binaries now target glibc 2.17.
