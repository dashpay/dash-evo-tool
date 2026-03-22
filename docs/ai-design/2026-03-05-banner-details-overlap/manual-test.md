# Manual Test: Banner Details Overlap Fix (#681)

## Prerequisites
- Developer mode enabled (to see "Show details" links on error banners)
- Ability to trigger multiple backend errors (e.g., invalid network config, expired identity operations)

## Test Scenario: Multiple Expanded Details

1. Trigger 2+ error banners that include technical details (e.g., attempt operations on a disconnected network, or perform actions that produce different errors in sequence)
2. Verify all banners appear stacked vertically without overlap
3. Click "Show details" on the **first** banner — verify the details section expands inline, pushing subsequent banners down
4. Click "Show details" on the **second** banner — verify its details section also expands without overlapping the first
5. Scroll within each details section independently — verify scroll positions are independent (no shared state)
6. Click "Hide details" on one banner — verify only that banner's details collapse; the other remains expanded
7. Dismiss one banner — verify remaining banners reflow correctly

## Expected Result
- Each banner's details section occupies its own vertical space
- No visual overlap between expanded details of different banners
- Scroll areas within each details section are independent

## Regression Check
- Single banner with "Show details" still works as before
- Banners without details are unaffected
