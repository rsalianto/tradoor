import { assert } from 'jsr:@std/assert';
import { csp } from './site.ts';

Deno.test('the policy allows scripts from this origin and Cloudflare\'s analytics host alone', () => {
  assert(csp().includes("script-src 'self' 'wasm-unsafe-eval' https://static.cloudflareinsights.com;"));
  assert(!csp().includes('google'), 'Google is gone from it');
  assert(!csp().includes("'unsafe-inline' https") && !/script-src[^;]*'unsafe-inline'/.test(csp()), 'no inline script');
  assert(csp().includes('connect-src') && csp().includes('https:'), 'its reports go out over https, already allowed');
});
