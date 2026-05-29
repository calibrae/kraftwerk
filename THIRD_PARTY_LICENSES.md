# Third-party libraries bundled with Kraftwerk

The macOS bundle (`Kraftwerk.app/Contents/Frameworks/`) embeds the
following dynamic libraries unmodified from Homebrew. Each retains
its upstream license; this file enumerates them as required by the
LGPL "library notice" obligation and the Apache-2.0 NOTICE obligation.

Linux `.deb` / `.rpm` users get these libraries from their distro's
package manager, not from kraftwerk's bundle, so this file only
applies to the macOS `.dmg` distribution.

| Library | Version policy | License | Upstream |
|---|---|---|---|
| libvirt | Latest stable Homebrew | LGPL-2.1-or-later | https://gitlab.com/libvirt/libvirt |
| glib (libglib, libgobject, libgio, libgmodule) | Latest stable Homebrew | LGPL-2.1-or-later | https://gitlab.gnome.org/GNOME/glib |
| gnutls | Latest stable Homebrew | LGPL-2.1-or-later | https://gitlab.com/gnutls/gnutls |
| libtasn1 | Latest stable Homebrew | LGPL-2.1-or-later | https://gitlab.com/gnutls/libtasn1 |
| gettext (libintl) | Latest stable Homebrew | LGPL-2.1-or-later | https://savannah.gnu.org/projects/gettext |
| p11-kit | Latest stable Homebrew | BSD-3-Clause (with some LGPL parts) | https://github.com/p11-glue/p11-kit |
| nettle (libnettle, libhogweed) | Latest stable Homebrew | LGPL-3-or-later (also dual-licensed GPL-2+) | https://www.lysator.liu.se/~nisse/nettle |
| gmp | Latest stable Homebrew | LGPL-3-or-later (dual GPL-2+) | https://gmplib.org |
| libidn2 | Latest stable Homebrew | LGPL-3-or-later (dual GPL-2+) | https://gitlab.com/libidn/libidn2 |
| libunistring | Latest stable Homebrew | LGPL-3-or-later | https://savannah.gnu.org/projects/libunistring |
| pcre2 (libpcre2-8) | Latest stable Homebrew | BSD-3-Clause | https://github.com/PCRE2Project/pcre2 |
| libssh2 | Latest stable Homebrew | BSD-3-Clause | https://www.libssh2.org |
| json-c | Latest stable Homebrew | MIT | https://github.com/json-c/json-c |
| OpenSSL (libcrypto.3, libssl.3) | Latest stable Homebrew | Apache-2.0 | https://www.openssl.org |

## License compliance notes

- **All bundled libraries are dynamically linked**, never statically.
  The bundle ships them as `.dylib` files in `Contents/Frameworks/`
  with install-name rewrites to `@executable_path/../Frameworks/…`.
  This means users may replace any embedded library with a compatible
  version of their own, satisfying the LGPL's "user modification"
  requirement.

- **No upstream sources are modified.** All libraries are vendored
  verbatim from the Homebrew bottle / formula at build time.
  Re-deriving the same binaries from upstream tarballs reproduces
  what's in the bundle.

- **License texts** for LGPL-2.1, LGPL-3.0, Apache-2.0, BSD-3-Clause,
  and MIT are reproduced in `LICENSES/` in this repository and shipped
  inside the macOS bundle at `Kraftwerk.app/Contents/Resources/LICENSES/`.

- **Source availability** for the LGPL components: every library is
  available at its upstream URL listed above. Kraftwerk's
  redistributor (the GitHub releases page) does not host these
  sources directly; the upstream links discharge the LGPL
  obligation of providing access to the corresponding source.

## Kraftwerk itself

Kraftwerk is licensed under MIT OR Apache-2.0 at the user's choice.
See `LICENSE-MIT` and `LICENSE-APACHE` at the repository root.
