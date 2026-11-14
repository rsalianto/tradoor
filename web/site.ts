/**
 * What the built page tells search engines and browsers about the site:
 * absolute URLs for its canonical address and preview image, robots.txt and a
 * sitemap — all from `SITE_URL` (`https://example.com`), set where the image
 * is built — and a Content Security Policy.
 *
 * Without `SITE_URL` the page still builds: the preview image stays relative
 * and robots.txt names no sitemap.
 *
 * The page carries no analytics of its own. A site on Cloudflare counts its
 * visits with Cloudflare Web Analytics, whose tag Cloudflare adds to each page
 * itself (Automatic setup): the policy allows that one host for it.
 */
import type { Plugin } from 'vite';

declare const process: { env: Record<string, string | undefined> };

/**
 * Scripts only from this origin and Cloudflare's analytics host, WebAssembly
 * compiled, data from the exchanges over TLS. Styles inline because Solid sets
 * them on elements. The page may not be framed (nginx's `frame-ancestors`; a
 * meta tag cannot say it). The same policy as deploy/nginx/security-headers.conf.
 */
export function csp(): string {
  // Cloudflare's tag reports to cloudflareinsights.com over https, which connect-src allows already.
  return [
    "default-src 'self'",
    "script-src 'self' 'wasm-unsafe-eval' https://static.cloudflareinsights.com",
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

export function siteUrl(raw: string | undefined): string {
  const url = (raw ?? '').trim().replace(/\/+$/, '');
  if (url && !/^https:\/\/[a-z0-9.-]+(:\d+)?$/i.test(url)) throw new Error(`SITE_URL must be the site's https origin, like https://example.com — got ${raw}`);
  return url;
}

export function site(): Plugin {
  const url = siteUrl(process.env.SITE_URL);
  return {
    name: 'tradoor-site',
    transformIndexHtml: {
      order: 'post',
      handler(html, ctx) {
        // Development runs Vite's own client, which a policy would have to allow.
        if (ctx.server) return html;
        let out = html.replace('<meta charset="UTF-8" />', `<meta charset="UTF-8" />\n    <meta http-equiv="Content-Security-Policy" content="${csp()}" />`);
        if (url) {
          // Each page its own address: the app at the root, the showcase at its file.
          const page = ctx.path === '/index.html' ? '/' : ctx.path;
          out = out
            .replaceAll('content="/og.png"', `content="${url}/og.png"`)
            .replace('<meta property="og:type"', `<link rel="canonical" href="${url}${page}" />\n    <meta property="og:url" content="${url}${page}" />\n    <meta property="og:type"`);
        }
        return out;
      },
    },
    generateBundle() {
      const robots = ['User-agent: *', 'Allow: /', 'Disallow: /api/', 'Disallow: /data/', ...(url ? ['', `Sitemap: ${url}/sitemap.xml`] : [])].join('\n');
      this.emitFile({ type: 'asset', fileName: 'robots.txt', source: `${robots}\n` });
      if (url) {
        const sitemap = `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n  <url><loc>${url}/</loc><changefreq>weekly</changefreq><priority>1.0</priority></url>\n  <url><loc>${url}/community.html</loc><changefreq>daily</changefreq><priority>0.7</priority></url>\n  <url><loc>${url}/doc.html</loc><changefreq>monthly</changefreq><priority>0.5</priority></url>\n</urlset>\n`;
        this.emitFile({ type: 'asset', fileName: 'sitemap.xml', source: sitemap });
      }
    },
  };
}
