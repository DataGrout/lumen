Lumen for Windows — Quick Start
================================

Lumen watches your LLM usage by running a small local proxy and pointing your
AI client (Claude Desktop, Cursor) at it. This bundle sets all of that up for
you.

WHAT'S IN THIS FOLDER
  run.bat                              <- double-click this
  lumen.ps1                            (the launcher run.bat calls)
  lumen-core-<version>-x86_64-windows.exe   (the Lumen daemon)


QUICK START
-----------
1. Keep all three files together in the same folder.
2. Double-click  run.bat
3. First run only: Windows SmartScreen may warn the .exe is unsigned. Click
   "More info" -> "Run anyway".

That's it. run.bat starts the Lumen daemon (if it isn't already running),
confirms the proxy is actually listening, finds Claude Desktop, and launches it
wired up to Lumen. Your usage then shows in the dashboard.

Dashboard:  http://127.0.0.1:9091/dashboard


LAUNCH CURSOR INSTEAD OF CLAUDE
-------------------------------
Open a terminal in this folder and run:
    run.bat -Client Cursor

Or just start/verify the daemon without launching a client:
    run.bat -Client None


IF IT SAYS THE PROXY ISN'T LISTENING
------------------------------------
On Windows a port can be unusable even when nothing is "using" it, because
Hyper-V / WSL2 reserve blocks of ports. Lumen defaults to port 9090. If that
port is reserved, the launcher will tell you, and you can pick another:

    run.bat -ProxyPort 9490

To see whether 9090 is in a reserved range:
    netsh interface ipv4 show excludedportrange protocol=tcp
    netstat -ano | findstr :9090


PREFER TO DO IT BY HAND?
------------------------
1. Start the daemon:      .\lumen-core-<version>-x86_64-windows.exe
2. In your client's shell, set the proxy + trust Lumen's CA, then launch it:

   $env:HTTPS_PROXY="http://127.0.0.1:9090"
   $env:NODE_EXTRA_CA_CERTS="$env:USERPROFILE\.lumen\ca.pem"
   & "<path to Claude.exe or Cursor.exe>"

The CA file is created automatically at %USERPROFILE%\.lumen\ca.pem the first
time the daemon runs.


STOPPING LUMEN
--------------
Close the client, then stop the daemon from a terminal:
    Invoke-RestMethod -Method Post http://127.0.0.1:9091/shutdown
(or end the "lumen-core" process in Task Manager).
