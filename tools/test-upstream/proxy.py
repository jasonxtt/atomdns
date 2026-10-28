#!/usr/bin/env python3
"""Per-test-namespace SOCKS5 CONNECT, restricted to frozen fixture endpoints.

Uses the same bounded bidirectional-copy lifecycle as tools/e2e-1/mock.py.
Run in the SUT namespace so upstream-veth delays apply to proxied transports.
"""
import argparse
import asyncio
import json
import pathlib
import socket
import struct
from fixture import IP


def allowed_target(host, port, ports):
    return host in {IP,'localhost'} and port in ports


async def run(manifest, output):
    ports={p for e in manifest['endpoints'] for p in e['ports'].values()}
    counts=dict(accepted=0,allowed=0,denied=0,active=0)
    async def handle(reader,writer):
        remote=None;counts['accepted']+=1;counts['active']+=1
        try:
            version,n=await reader.readexactly(2);methods=await reader.readexactly(n)
            if version!=5 or 0 not in methods:return
            writer.write(b'\x05\x00');await writer.drain()
            version,command,reserved,kind=await reader.readexactly(4)
            if version!=5 or command!=1 or reserved:return
            if kind==1:host=socket.inet_ntoa(await reader.readexactly(4))
            elif kind==3:host=(await reader.readexactly((await reader.readexactly(1))[0])).decode('ascii')
            else:return
            port=struct.unpack('!H',await reader.readexactly(2))[0]
            if not allowed_target(host,port,ports):
                counts['denied']+=1;writer.write(b'\x05\x02\x00\x01'+b'\0'*6);await writer.drain();return
            rr,remote=await asyncio.open_connection(IP,port);counts['allowed']+=1
            writer.write(b'\x05\x00\x00\x01'+socket.inet_aton(IP)+b'\0\0');await writer.drain()
            async def copy(source,target):
                while block:=await source.read(65536):target.write(block);await target.drain()
            tasks=[asyncio.create_task(copy(reader,remote)),asyncio.create_task(copy(rr,writer))]
            try:await asyncio.wait(tasks,return_when=asyncio.FIRST_COMPLETED)
            finally:
                for task in tasks:task.cancel()
                await asyncio.gather(*tasks,return_exceptions=True)
        except (asyncio.IncompleteReadError,ConnectionError,OSError,UnicodeError):pass
        finally:
            counts['active']-=1
            for stream in [writer,remote]:
                if stream:
                    stream.close()
                    try:await stream.wait_closed()
                    except (ConnectionError,OSError):pass
    server=await asyncio.start_server(handle,'127.0.0.1',17891)
    async def snapshots():
        while True:
            output.write_text(json.dumps(counts));await asyncio.sleep(1)
    task=asyncio.create_task(snapshots())
    try:
        async with server:await server.serve_forever()
    finally:task.cancel();await asyncio.gather(task,return_exceptions=True);output.write_text(json.dumps(counts))

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--manifest',type=pathlib.Path,required=True);p.add_argument('--stats',type=pathlib.Path,required=True);a=p.parse_args()
    asyncio.run(run(json.loads(a.manifest.read_text()),a.stats))
