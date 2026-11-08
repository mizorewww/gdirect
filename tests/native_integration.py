#!/usr/bin/env python3
"""Offline TLS/range integration tests against the actual patched libaria2."""
import hashlib
import http.server
import os
from pathlib import Path
import signal
import socket
import ssl
import subprocess
import sys
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parents[1]
DATA = bytes(range(256)) * 32768
requests = []
snis = []

class Server6(http.server.ThreadingHTTPServer):
    address_family = socket.AF_INET6

class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        requests.append((self.path, self.headers.get('Range'), self.headers.get('Authorization'), self.headers.get('Host')))
        if self.path == '/redirect':
            self.send_response(302)
            self.send_header('Location', 'https://example.com/steal')
            self.send_header('Content-Length', '0')
            self.end_headers()
            return
        if self.path == '/media':
            self.send_response(302)
            self.send_header('Location', f'https://drive.usercontent.google.com:{self.server.server_port}/blob')
            self.send_header('Content-Length', '0')
            self.end_headers()
            return
        start, end = 0, len(DATA)-1
        if self.headers.get('Range'):
            left, right = self.headers['Range'].removeprefix('bytes=').split('-')
            start, end = int(left), int(right) if right else end
            end = min(end, len(DATA)-1)
            self.send_response(206)
            self.send_header('Content-Range', f'bytes {start}-{end}/{len(DATA)}')
        else:
            self.send_response(200)
        self.send_header('Accept-Ranges', 'bytes')
        self.send_header('Content-Type', 'application/octet-stream')
        self.send_header('Content-Length', str(end-start+1))
        self.end_headers()
        try:
            for i in range(start, end+1, 65536):
                self.wfile.write(DATA[i:min(i+65536,end+1)])
                self.wfile.flush()
                if self.path == '/slow':
                    time.sleep(.025)
        except (BrokenPipeError, ConnectionResetError, ssl.SSLError):
            pass

def main():
    with tempfile.TemporaryDirectory(prefix='gdirect-test-') as tmp:
        tmp = Path(tmp)
        # Portable releases must work without the build machine's OpenSSL modules.
        os.environ['OPENSSL_MODULES'] = str(tmp / 'no-openssl-modules')
        ssl_prefix = os.environ.get('OPENSSL_PREFIX')
        if sys.platform == 'darwin' and not ssl_prefix:
            ssl_prefix = subprocess.check_output(['brew','--prefix','openssl@3'],text=True).strip()
        harness = tmp/'main.cc'
        harness.write_text('''#include <cstdint>
#include <cstdlib>
#include <cstdio>
#include <string>
#include <thread>
extern "C" int gdirect_download(const char*,const char*,const char*,const char*,const char*,const char*,const char*,int) noexcept;
using Callback=int(*)(int64_t,int64_t,int,int,void*);
extern "C" int gdirect_download_ex(const char*,const char*,const char*,const char*,const char*,const char*,const char*,int,Callback,void*,bool,unsigned) noexcept;
int progress(int64_t done,int64_t total,int speed,int connections,void* data){
  ++*static_cast<int*>(data);return std::getenv("GDIRECT_TEST_CANCEL") && done>0;
}
int main(int n,char**v){
 if(n!=8)return 99;
 const char* ip=std::getenv("GDIRECT_TEST_IP");if(!ip)ip="127.0.0.1";
 if(std::getenv("GDIRECT_TEST_REPEAT")){
  // browse runs one download after another, each on a fresh worker thread.
  for(int i=0;i<3;i++){
   std::string name=std::string(v[3])+"."+std::to_string(i);int result=99,count=0;
   std::thread worker([&]{result=gdirect_download_ex(v[1],v[2],name.c_str(),ip,v[4],v[5],v[6],v[7][0]-'0',progress,&count,false,0);});
   worker.join();if(result)return result;
  }
  return 0;
 }
 if(!std::getenv("GDIRECT_TEST_CALLBACK"))return gdirect_download(v[1],v[2],v[3],ip,v[4],v[5],v[6],v[7][0]-'0');
 int result=99,count=0;
 std::thread worker([&]{result=gdirect_download_ex(v[1],v[2],v[3],ip,v[4],v[5],v[6],v[7][0]-'0',progress,&count,false,0);});
 worker.join();std::printf("callbacks=%d\\n",count);return count>0 ? result : 98;
}
''')
        cmd = ['c++','-std=c++11',str(harness),str(ROOT/'native/bridge.cc'),'-I'+str(ROOT/'target/libaria2/include'),str(ROOT/'target/libaria2/lib/libaria2.a')]
        if ssl_prefix:
            cmd += ['-L'+ssl_prefix+'/lib']
        if sys.platform == 'darwin':
            cmd += ['-framework','Security']
        cmd += ['-pthread','-lssl','-lcrypto']
        if sys.platform == 'win32':
            cmd += ['-lws2_32','-lwsock32','-lgdi32','-lwinmm','-liphlpapi','-lpsapi','-lcrypt32','-lbcrypt','-ladvapi32']
        cmd += ['-o',str(tmp/'harness')]
        subprocess.run(cmd,check=True)
        # The certificate covers both the real hosts and the forged identity:
        # in forged mode gdirect connects and verifies as g.cn while Host: keeps
        # the real hostname, so the fixture must certify all of them.
        subprocess.run(['openssl','req','-x509','-newkey','rsa:2048','-nodes','-days','1','-subj','/CN=drive.usercontent.google.com','-addext','subjectAltName=DNS:drive.usercontent.google.com,DNS:www.googleapis.com,DNS:g.cn','-keyout',str(tmp/'key.pem'),'-out',str(tmp/'cert.pem')],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        server = http.server.ThreadingHTTPServer(('127.0.0.1',0), Handler)
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(tmp/'cert.pem',tmp/'key.pem')
        context.set_servername_callback(lambda sock,name,ctx:snis.append(name))
        server.socket = context.wrap_socket(server.socket,server_side=True)
        threading.Thread(target=server.serve_forever,daemon=True).start()
        def command(name,path='/blob',host='drive.usercontent.google.com',sni='g.cn',token='',ca=None):
            return [str(tmp/'harness'),f'https://{host}:{server.server_port}{path}',str(tmp),name,sni,token,str(ca or tmp/'cert.pem'),'4']
        def run(*args,success=True,**kwargs):
            p=subprocess.run(command(*args,**kwargs),capture_output=True,text=True,timeout=30)
            assert (p.returncode == 0) == success, (p.returncode,p.stdout,p.stderr)
        def check(name):
            assert hashlib.sha256((tmp/name).read_bytes()).digest()==hashlib.sha256(DATA).digest()
        try:
            run('split.bin'); check('split.bin')
            assert 'g.cn' in snis and any(r[1] for r in requests)
            assert all(r[3] == 'drive.usercontent.google.com' for r in requests)
            print('PASS: pinned IP, forged SNI identity, real Host header, parallel ranges and byte integrity')
            run('no-sni.bin',sni='');check('no-sni.bin');assert None in snis
            print('PASS: no-SNI with certificate verification')
            # With no forged identity the certificate is checked against the
            # URL's own hostname; the fixture cert does not cover drive.google.com.
            run('mismatch.bin',host='drive.google.com',sni='',success=False)
            print('PASS: rejects wrong original hostname despite successful TLS transport')
            run('redirect.bin',path='/redirect',success=False)
            print('PASS: rejects non-Google redirects')
            requests.clear()
            run('media.bin',path='/media',host='www.googleapis.com',token='fixture-token');check('media.bin')
            assert any(p=='/media' and t=='Bearer fixture-token' for p,_,t,_ in requests)
            assert all(t is None for p,_,t,_ in requests if p=='/blob')
            assert all(h == 'drive.usercontent.google.com' for p,_,_,h in requests if p == '/blob')
            print('PASS: OAuth token is stripped from media redirects')
            if sys.platform != 'win32':
                p=subprocess.Popen(command('resume.bin',path='/slow'),stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
                time.sleep(.4);p.send_signal(signal.SIGINT);p.wait(timeout=15)
                assert (tmp/'resume.bin.aria2').exists(), 'missing resume control file'
                requests.clear()
                run('resume.bin',path='/slow');check('resume.bin')
                assert any(r[1] and r[1] != 'bytes=0-' for r in requests)
                assert not (tmp/'resume.bin.aria2').exists()
                print('PASS: interrupted download resumes with complete byte integrity')
            env=dict(os.environ, GDIRECT_TEST_CALLBACK='1')
            p=subprocess.run(command('callback.bin',path='/slow'),env=env,capture_output=True,text=True,timeout=30)
            assert p.returncode==0 and 'callbacks=' in p.stdout, (p.returncode,p.stdout,p.stderr)
            check('callback.bin')
            p=subprocess.run(command('cancel.bin',path='/slow'),env=dict(env,GDIRECT_TEST_CANCEL='1'),capture_output=True,text=True,timeout=30)
            assert p.returncode not in (0,98) and (tmp/'cancel.bin.aria2').exists(), (p.returncode,p.stdout,p.stderr)
            run('cancel.bin',path='/slow');check('cancel.bin')
            print('PASS: worker-thread progress callback, cancellation and resume')
            p=subprocess.run(command('repeat.bin'),env=dict(os.environ,GDIRECT_TEST_REPEAT='1'),capture_output=True,text=True,timeout=60)
            assert p.returncode==0, (p.returncode,p.stdout,p.stderr)
            for i in range(3):
                check(f'repeat.bin.{i}')
            print('PASS: several downloads in one process')
            try:
                server6 = Server6(('::1',0), Handler)
            except OSError as e:
                print(f'SKIP: IPv6 loopback unavailable ({e})')
            else:
                server6.socket = context.wrap_socket(server6.socket,server_side=True)
                threading.Thread(target=server6.serve_forever,daemon=True).start()
                try:
                    cmd=command('v6.bin')
                    cmd[1]=f'https://drive.usercontent.google.com:{server6.server_port}/blob'
                    p=subprocess.run(cmd,env=dict(os.environ,GDIRECT_TEST_IP='::1'),capture_output=True,text=True,timeout=30)
                    assert p.returncode==0, (p.returncode,p.stdout,p.stderr)
                    check('v6.bin')
                    print('PASS: pinned IPv6 address')
                finally:
                    server6.shutdown();server6.server_close()
        finally:
            server.shutdown();server.server_close()

if __name__ == '__main__':
    main()
