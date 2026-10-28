import { assert, assertEquals, assertThrows } from 'jsr:@std/assert';
import { beaconTag, beaconToken, csp } from './site.ts';

Deno.test('a Cloudflare token is 32 hex digits, or none at all', () => {
  assertEquals(beaconToken(undefined), '');
  assertEquals(beaconToken('  '), '');
  assertEquals(beaconToken('55A3961483124A5EA76D2E37D07C1744'), '55a3961483124a5ea76d2e37d07c1744');
  assertThrows(() => beaconToken('G-ABC123DEF4'), Error, 'CF_BEACON');
  assertThrows(() => beaconToken('55a3961483124a5ea76d2e37d07c174'), Error, 'CF_BEACON');
});

Deno.test('the policy names the beacon host only when there is a beacon', () => {
  assert(!csp(false).includes('cloudflareinsights'), 'no analytics, no third party');
  assert(csp(true).includes("script-src 'self' 'wasm-unsafe-eval' https://static.cloudflareinsights.com"));
  assert(!csp(true).includes('google'), 'Google is gone from it');
  assert(csp(true).includes('connect-src') && csp(true).includes('https:'), 'its reports go out over https, already allowed');
});

Deno.test('the beacon is Cloudflare’s own snippet for the token', () => {
  assertEquals(
    beaconTag('55a3961483124a5ea76d2e37d07c1744'),
    `<script type="module" src="https://static.cloudflareinsights.com/beacon.min.js" data-cf-beacon='{"token": "55a3961483124a5ea76d2e37d07c1744"}'></script>`,
  );
});
