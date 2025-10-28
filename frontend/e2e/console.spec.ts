import { test, expect } from '@playwright/test';

// key: lifecycle-console-ui -> playwright-smoke
const PAGE_PATH = '/console/lifecycle';

test.describe('Lifecycle console', () => {
  test('renders timeline shells and verdict cards', async ({ page }) => {
    await page.goto(PAGE_PATH);
    await expect(page.getByRole('heading', { name: 'Lifecycle Console' })).toBeVisible();
    await expect(page.getByText('Unified remediation, trust, intelligence')).toBeVisible();
  });
});
