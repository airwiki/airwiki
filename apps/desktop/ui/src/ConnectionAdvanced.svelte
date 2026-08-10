<script lang="ts">
  import type { LanRuntimeSummary } from './api';
  import type { MessageArgs } from './i18n';

  export let lanRuntime: LanRuntimeSummary | null;
  export let peerId: string;
  export let address: string;
  export let blockedPublishers: string[];
  export let busy: boolean;
  export let t: (id: string, args?: MessageArgs) => string;
  export let onpeerid: (value: string) => void;
  export let onaddress: (value: string) => void;
  export let onadd: () => void;
  export let onremove: () => void;
  export let onunblock: (publisherId: string) => void;

  function stateLabel(state: string): string {
    if (state === 'listening' || state === 'active') return t('status-ready');
    if (state === 'starting') return t('status-working');
    if (state === 'failed') return t('status-needs-attention');
    return t('status-optional-disabled');
  }
</script>

<details class="advanced-disclosure connection-advanced" aria-busy={busy}>
  <summary>{t('desktop-advanced-details')}</summary>
  {#if lanRuntime}
    <dl>
      <div><dt>{t('desktop-listener')}</dt><dd>{stateLabel(lanRuntime.listener)}</dd></div>
      <div><dt>{t('desktop-discovery')}</dt><dd>{stateLabel(lanRuntime.discovery)}</dd></div>
      <div><dt>{t('desktop-interfaces')}</dt><dd>{lanRuntime.addressCount}</dd></div>
    </dl>
  {/if}
  <section>
    <h3>{t('desktop-community-indexes')}</h3>
    <p>{t('desktop-community-indexes-body')}</p>
    <label><span>{t('desktop-peer-id')}</span><input value={peerId} oninput={(event) => onpeerid(event.currentTarget.value)} maxlength="200" /></label>
    <label><span>{t('desktop-multiaddress')}</span><input value={address} oninput={(event) => onaddress(event.currentTarget.value)} maxlength="500" /></label>
    <div class="row-actions"><button class="secondary" onclick={onadd} disabled={!peerId.trim() || !address.trim()}>{t('search-public-index-add')}</button><button class="text-action" onclick={onremove} disabled={!peerId.trim()}>{t('search-public-index-remove')}</button></div>
  </section>
  {#if blockedPublishers.length}
    <section><h3>{t('desktop-blocked-publishers')}</h3>{#each blockedPublishers as publisherId (publisherId)}<div class="blocked-publisher"><code>{publisherId.slice(0, 16)}…</code><button class="text-action" onclick={() => onunblock(publisherId)}>{t('search-public-unblock-publisher')}</button></div>{/each}</section>
  {/if}
</details>
