import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import Checkbox from './Checkbox.svelte';
import SelectField from './SelectField.svelte';
import Switch from './Switch.svelte';
import TextField from './TextField.svelte';

describe('desktop form controls', () => {
  afterEach(cleanup);

  it('associates text field copy and reports edited values', async () => {
    const oninput = vi.fn();
    render(TextField, { label: 'Wiki name', description: 'Visible to nearby devices', value: '', oninput });

    const field = screen.getByRole('textbox', { name: 'Wiki name' });
    expect(field).toHaveAccessibleDescription('Visible to nearby devices');
    await fireEvent.input(field, { target: { value: 'Atlas' } });
    expect(oninput).toHaveBeenCalledWith('Atlas');
  });

  it('binds select values through one labeled control', async () => {
    render(SelectField, {
      label: 'Appearance',
      value: 'system',
      options: [{ value: 'system', label: 'Follow system' }, { value: 'dark', label: 'Dark' }]
    });

    const select = screen.getByRole('combobox', { name: 'Appearance' });
    await fireEvent.change(select, { target: { value: 'dark' } });
    expect(select).toHaveValue('dark');
  });

  it('uses a checkbox for explicit confirmation', async () => {
    const onchange = vi.fn();
    render(Checkbox, { label: 'I reviewed the evidence', checked: false, onchange });

    await fireEvent.click(screen.getByRole('checkbox', { name: 'I reviewed the evidence' }));
    expect(onchange).toHaveBeenCalledWith(true);
  });

  it('exposes reversible preferences as a semantic switch', async () => {
    const onchange = vi.fn();
    render(Switch, { label: 'Check for updates automatically', checked: false, onchange });

    const control = screen.getByRole('switch', { name: 'Check for updates automatically' });
    await fireEvent.click(control);
    expect(onchange).toHaveBeenCalledWith(true);
  });
});
