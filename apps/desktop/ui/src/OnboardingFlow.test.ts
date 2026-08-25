import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import axe from 'axe-core';
import { afterEach, describe, expect, it, vi } from 'vitest';
import OnboardingFlow from './OnboardingFlow.svelte';
import { readySnapshot } from './test/fixtures';

function onboardingProps() {
  const snapshot = readySnapshot();
  snapshot.wikis = [];
  return {
    snapshot,
    locale: 'es' as const,
    modelLicensesConfirmed: false,
    actionBusy: false,
    actionMessage: '',
    onpickfolder: vi.fn(async () => ({ token: 'synthetic-folder-token', displayName: 'Apuntes' })),
    oncreatewiki: vi.fn(async () => undefined),
    onprepare: vi.fn(),
    onfinish: vi.fn()
  };
}

describe('OnboardingFlow', () => {
  afterEach(cleanup);

  it('moves focus with each step and makes postponing the first Wiki explicit', async () => {
    const { container } = render(OnboardingFlow, onboardingProps());

    await fireEvent.click(screen.getByRole('button', { name: 'Continuar' }));
    const folderHeading = screen.getByRole('heading', { name: 'Agregar tu primera wiki' });
    await waitFor(() => expect(folderHeading).toHaveFocus());
    expect(screen.getByRole('button', { name: 'Continuar' })).toBeDisabled();

    await fireEvent.click(screen.getByRole('button', { name: 'Continuar sin una carpeta' }));
    expect(screen.getByRole('button', { name: 'Continuar' })).toBeEnabled();
    await fireEvent.click(screen.getByRole('button', { name: 'Continuar' }));
    await waitFor(() => expect(screen.getByRole('heading', { name: 'AirWiki está listo' })).toHaveFocus());
    expect(screen.getByText(/Agrega una carpeta desde el estado vacío/)).toBeInTheDocument();

    const report = await axe.run(container, { rules: { 'color-contrast': { enabled: false } } });
    expect(report.violations.filter((violation) => ['critical', 'serious'].includes(violation.impact ?? ''))).toEqual([]);
  });

  it('creates the first folder Wiki from the setup flow', async () => {
    const props = onboardingProps();
    render(OnboardingFlow, props);
    await fireEvent.click(screen.getByRole('button', { name: 'Continuar' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Elegir carpeta…' }));

    expect(await screen.findByRole('textbox', { name: 'Nombre de la wiki' })).toHaveValue('Apuntes');
    await fireEvent.click(screen.getByRole('button', { name: 'Crear wiki' }));

    await waitFor(() => expect(props.oncreatewiki).toHaveBeenCalledWith('Apuntes', 'synthetic-folder-token', true));
    expect(screen.getByText('La primera carpeta ya está vinculada.')).toBeInTheDocument();
  });
});
