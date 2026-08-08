<script lang="ts">
  import { BookOpen, CheckCircle2, Search, Settings2, Sparkles } from '@lucide/svelte';

  type Destination = 'library' | 'review' | 'search' | 'system';

  const destinations = [
    { id: 'library', label: 'Biblioteca', icon: BookOpen },
    { id: 'review', label: 'Revisión', icon: CheckCircle2 },
    { id: 'search', label: 'Buscar', icon: Search },
    { id: 'system', label: 'Sistema', icon: Settings2 }
  ] as const;

  let destination: Destination = 'library';

  function select(next: Destination) {
    destination = next;
    window.location.hash = next;
  }
</script>

<svelte:head><meta name="theme-color" content="#07131f" /></svelte:head>

<div class="shell">
  <aside class="rail" aria-label="Navegación principal">
    <div class="brand"><span class="brand-mark">A</span><span>AirWiki</span></div>
    <nav>
      {#each destinations as item}
        <button class:active={destination === item.id} onclick={() => select(item.id)}>
          <item.icon size={18} strokeWidth={1.8} aria-hidden="true" />
          <span>{item.label}</span>
        </button>
      {/each}
    </nav>
    <div class="device-state">
      <span class="pulse" aria-hidden="true"></span>
      <div><strong>En este dispositivo</strong><small>Preparando servicios privados</small></div>
    </div>
  </aside>

  <main>
    <header>
      <div><p class="eyebrow">Espacio de conocimiento local</p><h1>Tu evidencia, lista para comprobar.</h1></div>
      <button class="primary"><Sparkles size={17} />Siguiente acción</button>
    </header>

    <section class="workspace" aria-live="polite">
      <div class="evidence-rail" aria-hidden="true"><i></i><i></i><i></i></div>
      <div class="content">
        <p class="section-label">{destinations.find((item) => item.id === destination)?.label}</p>
        <h2>{destination === 'library' ? 'De archivos locales a conocimiento verificable' : destinations.find((item) => item.id === destination)?.label}</h2>
        <p class="lede">AirWiki mantiene cada afirmación unida a su fuente, su revisión humana y sus permisos de publicación.</p>

        <div class="sequence">
          <article><span>Fuente</span><strong>Carpetas elegidas por ti</strong><p>Los originales permanecen donde están.</p></article>
          <article><span>Preparación</span><strong>Análisis local</strong><p>La IA propone; nunca publica ni concede acceso.</p></article>
          <article><span>Decisión</span><strong>Revisión pendiente</strong><p>Comprueba la evidencia antes de hacerla visible.</p></article>
        </div>
      </div>
    </section>
  </main>
</div>
