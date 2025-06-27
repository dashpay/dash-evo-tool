# SPV Peer Cycling Enhancement

## Overview

The SPV manager now includes enhanced peer cycling functionality that automatically switches to different peers when the current peer stops providing headers. This ensures continuous syncing even when individual peers become unresponsive.

## Key Features

### 1. Automatic Detection of Stuck Peers

The progress monitor checks sync status every 5 seconds and detects two scenarios:

- **Stuck at Height 0**: If no headers are received for 30 seconds (6 checks), the system assumes the peer cannot provide genesis headers
- **Stuck at Non-Zero Height**: If sync stops progressing at any height > 0 for 40 seconds (8 checks), the system assumes the peer has stopped responding

### 2. Intelligent Peer Group Rotation

- Peers are organized into groups of 3
- When stuck, the system automatically switches to the next peer group
- The sync continues from the last known height (no need to restart from genesis)
- Up to 10 peer groups are tried before giving up

### 3. Peer Sources

Peers are obtained from (in order of preference):
1. DAPI addresses from config (converted to P2P addresses)
2. Default hardcoded peers for the network

### 4. Progress Preservation

When switching peers:
- Current sync height is recorded
- SPV client is gracefully stopped
- New peer group is selected
- Client restarts and continues from the last height

## Testing

Use the enhanced test script to monitor peer cycling:

```bash
./test_peer_cycling.sh
```

The script provides color-coded output showing:
- ✓ Green: Sync progress
- ⚠ Yellow: No progress warnings
- ✗ Red: Stuck detection
- ↻ Blue: Peer switching
- ↗ Green: Sync resumption

## Configuration

- **Height 0 Timeout**: 30 seconds (6 checks × 5 seconds)
- **Non-Zero Height Timeout**: 40 seconds (8 checks × 5 seconds)
- **Peers per Group**: 3
- **Max Peer Groups**: 10

## Mainnet Considerations

For mainnet, the system includes special handling:
- Attempts to set a checkpoint at height 2,290,000
- Recognizes that most mainnet nodes only serve recent blocks
- Provides appropriate error messages when genesis sync is not possible

## Log Messages

Key log messages to watch for:

```
[PROGRESS MONITOR] No header sync progress at height X (check #Y)
[PROGRESS MONITOR] Header sync stuck at height X for Y seconds. Peer likely stopped responding.
Current sync height before switching peers: X
Switching to next peer group (attempt #Y) to continue sync from height X...
Continuing sync from height X with new peers
```

## Implementation Details

The enhancement is implemented in `src/components/spv_manager.rs`:

1. **Progress Monitoring**: `start_progress_updater()` tracks sync progress and detects stuck states
2. **Peer Switching**: `try_next_peer_group()` handles the peer rotation logic
3. **State Tracking**: Uses `peer_group_index` to track which group is active
4. **Graceful Handling**: Ensures clean shutdown and restart between peer switches