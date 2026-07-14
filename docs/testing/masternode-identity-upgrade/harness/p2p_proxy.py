#!/usr/bin/env python3
"""TCP proxy 127.0.0.1:19899 -> 127.0.0.1:20001 so DET PR887's SPV client
(hardcoded regtest p2p port 19899) can reach dashmate local_1 core (20001)."""
import asyncio

async def pipe(r, w):
    try:
        while True:
            data = await r.read(65536)
            if not data:
                break
            w.write(data)
            await w.drain()
    except Exception:
        pass
    finally:
        try:
            w.close()
        except Exception:
            pass

async def handle(client_r, client_w):
    try:
        remote_r, remote_w = await asyncio.open_connection('127.0.0.1', 20001)
    except Exception:
        client_w.close()
        return
    await asyncio.gather(pipe(client_r, remote_w), pipe(remote_r, client_w))

async def main():
    server = await asyncio.start_server(handle, '127.0.0.1', 19899)
    print('proxy listening on 19899 -> 20001', flush=True)
    async with server:
        await server.serve_forever()

asyncio.run(main())
