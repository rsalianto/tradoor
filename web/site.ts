/**
 * What the built page tells search engines and browsers about the site:
 * absolute URLs for its canonical address and preview image, robots.txt and a
 * sitemap — all from `SITE_URL` (`https://example.com`), set where the image
 * is built — and a Content Security Policy.
 *
 * Without `SITE_URL` the page still builds: the preview image stays relative
 * and robots.txt names no sitemap.
 *
 * `CF_BEACON`, set the same way, adds Cloudflare Web Analytics: the beacon of
 * the site's token, its 32 hex digits, which the policy allows by host. It
 * sets no cookie and keeps nothing on the visitor. Without it the page carries
 * no analytics and the policy stays as tight — a fork counts its visitors only
 * once it sets a token of its own.
 */
import type { Plugin } from 'vite';

declare const process: { env: Record<string, string | undefined> };

/**
 * Scripts only from this origin (and Cloudflare's beacon host, with analytics),
 * WebAssembly compiled, data from the exchanges over TLS. Styles inline because Solid sets them on elements. The page may
 * not be framed (nginx's `frame-ancestors`; a meta tag cannot say it).
 */
export function csp(analytics = false): string {
  // The beacon reports to cloudflareinsights.com over https, which connect-src allows already.
  const beacon = analytics ? ' https://static.cloudflareinsights.com' : '';
  return [
    "default-src 'self'",
    `script-src 'self' 'wasm-unsafe-eval'${beacon}`,
    "worker-src 'self'",
    "style-src 'self' 'unsafe-inline'",
    "img-src 'self' data: blob:",
    "font-src 'self' data:",
    "connect-src 'self' https: wss:",
    "manifest-src 'self'",
    "object-src 'none'",
    "base-uri 'self'",
    "form-action 'self'",
  ].join('; ');
}

/** A Cloudflare Web Analytics token, or empty; anything else is a mistake worth a failed build. */
export function beaconToken(raw: string | undefined): string {
  const token = (raw ?? '').trim().toLowerCase();
  if (token && !/^[0-9a-f]{32}$/.test(token)) throw new Error(`CF_BEACON must be a Cloudflare Web Analytics token, 32 hex digits — got ${raw}`);
  return token;
}

/** Cloudflare's snippet for the token, as its dashboard gives it. */
export function beaconTag(token: string): string {
  return `<script type="module" src="https://static.cloudflareinsights.com/beacon.min.js" data-cf-beacon='{"token": "${token}"}'></script>`;
}

export function siteUrl(raw: string | undefined): string {
  const url = (raw ?? '').trim().replace(/\/+$/, '');
  if (url && !/^https:\/\/[a-z0-9.-]+(:\d+)?$/i.test(url)) throw new Error(`SITE_URL must be the site's https origin, like https://example.com — got ${raw}`);
  return url;
}

export function site(): Plugin {
  const url = siteUrl(process.env.SITE_URL);
  const beacon = beaconToken(process.env.CF_BEACON);
  return {
    name: 'tradoor-site',
    transformIndexHtml: {
      order: 'post',
      handler(html, ctx) {
        // Development runs Vite's own client, which a policy would have to allow,
        // and its visits are nobody's to count.
        if (ctx.server) return html;
        let out = html.replace('<meta charset="UTF-8" />', `<meta charset="UTF-8" />\n    <meta http-equiv="Content-Security-Policy" content="${csp(!!beacon)}" />`);
        if (beacon) out = out.replace('</body>', `  ${beaconTag(beacon)}\n  </body>`);
        if (url) {
          out = out
            .replaceAll('content="/og.png"', `content="${url}/og.png"`)
            .replace('<meta property="og:type"', `<link rel="canonical" href="${url}/" />\n    <meta property="og:url" content="${url}/" />\n    <meta property="og:type"`);
        }
        return out;
      },
    },
    generateBundle() {
      const robots = ['User-agent: *', 'Allow: /', 'Disallow: /api/', 'Disallow: /data/', ...(url ? ['', `Sitemap: ${url}/sitemap.xml`] : [])].join('\n');
      this.emitFile({ type: 'asset', fileName: 'robots.txt', source: `${robots}\n` });
      if (url) {
        const sitemap = `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n  <url><loc>${url}/</loc><changefreq>weekly</changefreq><priority>1.0</priority></url>\n</urlset>\n`;
        this.emitFile({ type: 'asset', fileName: 'sitemap.xml', source: sitemap });
      }
    },
  };
}
