import subprocess, sys, time
import psutil

exe = sys.argv[1]
args = sys.argv[2:]
proc = subprocess.Popen([exe] + args, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
ps = psutil.Process(proc.pid)
peak = 0
t0 = time.perf_counter()
while proc.poll() is None:
    try:
        rss = ps.memory_info().rss
        if rss > peak:
            peak = rss
    except psutil.NoSuchProcess:
        break
    time.sleep(0.02)
elapsed = time.perf_counter() - t0
print(f"elapsed_s={elapsed:.2f} peak_rss_mb={peak/1048576:.0f}")
