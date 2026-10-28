/**
 * The exchanges to open an account on through tradoor's referral links: a
 * dialog anyone can open from the top bar or the phone's menu, and the card
 * that offers them once (lib/referral). A link opens the exchange's own page in
 * a new tab, carrying the referral code and nothing about the visitor.
 */
import { For, type Component } from 'solid-js';
import { Dynamic } from 'solid-js/web';
import { EXCHANGES, ExchangeLogo } from '../lib/exchanges';
import { Icon } from '../lib/icons';
import { dismissReferralCard } from '../lib/referral';

interface Partner {
  name: string;
  Logo: Component<{ size?: number }>;
  url: string;
  offer: string;
  /** A second line, for the partner the card leads with. */
  more?: string;
  /** Led with, on its own row above the rest. */
  lead?: boolean;
}

/** Nado's mark, from its site, on the black disc every exchange's mark sits on. tradoor does not chart Nado, so it is not among EXCHANGES. */
function NadoLogo(props: { size?: number }) {
  return (
    <svg class="xlogo" width={props.size ?? 14} height={props.size ?? 14} viewBox="0 0 256 256" aria-hidden="true">
      <circle cx="128" cy="128" r="128" fill="#000" />
      <path
        transform="translate(28 28) scale(0.78)"
        fill="#fff"
        d="M94.28 145.77L160.64 219.18L165.81 222H181.58L184.87 218.71V192.35L182.52 186.94L142.29 141.53C132.17 130.23 124.41 119.88 109.58 117.06L111.46 110.7L101.58 111.88L92.63 140.59L94.28 145.76V145.77ZM144.88 65.3L47.94 85.77L42.76 88.83L35 102.71L36.41 107.18L59.23 120.36L64.88 121.07L124.41 108.83C139.24 105.77 152.17 104.13 162.06 92.83L167.23 98.24L171.23 89.07L150.05 66.48L144.88 65.3V65.3ZM189.82 148.82L220.4 54.7V48.59L212.4 34.94L207.7 34L184.87 47.18L181.34 51.65L162.28 109.3C157.58 123.66 152.4 135.65 157.34 150L150.05 151.65L155.93 159.66L186.04 152.83L189.81 148.83Z"
      />
    </svg>
  );
}

const exchangeLogo =
  (id: 'bybit' | 'binance'): Component<{ size?: number }> =>
  (p) => <ExchangeLogo exchange={EXCHANGES[id]} size={p.size} />;

const PARTNERS: readonly Partner[] = [
  {
    name: 'Nado',
    Logo: NadoLogo,
    url: 'https://app.nado.xyz?join=tradoor',
    offer: 'Backed by the team behind Kraken',
    more: 'Self-custody perps and spot on Ink · up to 20× · orders matched in 5–15 ms',
    lead: true,
  },
  { name: 'Bybit', Logo: exchangeLogo('bybit'), url: 'https://www.bybitglobal.com/invite?ref=P5RO8W&medium=referral&utm_campaign=evergreen&share_to=post', offer: 'Perpetuals · spot · copy trading' },
  { name: 'Binance', Logo: exchangeLogo('binance'), url: 'https://www.bmwweb.biz/referral/earn-together/refer2earn-usdc/claim?hl=en&ref=GRO_28502_YJYU6&utm_source=referral_entrance', offer: 'Spot · futures · the deepest books' },
];

export function ReferralDialog(props: { onClose(): void }) {
  return (
    <div class="backdrop" onPointerDown={() => props.onClose()}>
      <div
        class="dialog narrow ref"
        tabIndex={-1}
        ref={(el) => queueMicrotask(() => el.focus())}
        onPointerDown={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key !== 'Escape') return;
          e.stopPropagation();
          props.onClose();
        }}
      >
        <header>
          <span class="title caps">Partner exchanges</span>
          <span class="spacer" />
          <button class="x" onClick={() => props.onClose()}>
            esc ✕
          </button>
        </header>
        <p class="ref-intro">
          Open an account through these links and tradoor earns a share of the fees, at no extra cost to you — it keeps the charts free. Bybit and Binance add their new-user rewards.
        </p>
        <div class="ref-list">
          <For each={PARTNERS}>
            {(p) => (
              <a class="ref-row" classList={{ lead: p.lead }} href={p.url} target="_blank" rel="sponsored noopener" onClick={dismissReferralCard}>
                <Dynamic component={p.Logo} size={28} />
                <span class="who">
                  <b>
                    {p.name}
                    {p.lead && <i class="new">New</i>}
                  </b>
                  <span>{p.offer}</span>
                  {p.more && <span>{p.more}</span>}
                </span>
                <span class="go">
                  Sign up <Icon name="external" size={11} stroke={2} />
                </span>
              </a>
            )}
          </For>
        </div>
        <footer>
          <span>Referral links · open in a new tab</span>
        </footer>
      </div>
    </div>
  );
}

export function ReferralCard() {
  return (
    <aside class="ref-card" aria-label="Partner exchanges">
      <header>
        <span class="eyebrow">
          <Icon name="gift" size={12} /> New to Nado, Bybit or Binance?
        </span>
        <button class="x" aria-label="Close" onClick={dismissReferralCard}>
          <Icon name="close" size={10} stroke={2.4} />
        </button>
      </header>
      <p>Sign up through tradoor's links — no extra cost, new-user rewards on Bybit and Binance, and it keeps the charts free.</p>
      <For each={PARTNERS.filter((p) => p.lead)}>
        {(p) => (
          <a class="lead" href={p.url} target="_blank" rel="sponsored noopener" onClick={dismissReferralCard}>
            <Dynamic component={p.Logo} size={24} />
            <span class="who">
              <b>
                {p.name}
                <i class="new">New</i>
              </b>
              <span>{p.offer}</span>
            </span>
            <Icon name="external" size={10} stroke={2} />
          </a>
        )}
      </For>
      <div class="pair">
        <For each={PARTNERS.filter((p) => !p.lead)}>
          {(p) => (
            <a href={p.url} target="_blank" rel="sponsored noopener" onClick={dismissReferralCard}>
              <Dynamic component={p.Logo} size={18} />
              {p.name}
              <Icon name="external" size={10} stroke={2} />
            </a>
          )}
        </For>
      </div>
      <footer>
        <button class="later" onClick={dismissReferralCard}>
          Not now
        </button>
      </footer>
    </aside>
  );
}
