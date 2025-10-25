import { test, expect } from '@playwright/test';

test('index page loads', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'MCP Host' })).toBeVisible();
});
