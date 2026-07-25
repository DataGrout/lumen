Lumen for Windows — Quick Start
================================

Lumen watches your LLM usage by running a small local proxy and pointing your
AI client (Claude Desktop, Cursor) at it. This bundle sets all of that up for
you.

WHAT'S IN THIS FOLDER
  run.bat            <- double-click this (auto-detects your installed AI client)
  run-claude.bat     launch Claude Desktop      run-cursor.bat   launch Cursor
  run-code.bat       launch Claude Code (CLI)   run-stop.bat     stop the daemon
  lumen.ps1          (the launcher the .bat files call)
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


LAUNCH A SPECIFIC CLIENT
------------------------
run.bat (no args) auto-detects and launches whatever you have installed. To
pick a specific one, double-click a shim (or run the equivalent command):
    run-claude.bat   Claude Desktop app   =  run.bat -Client Claude
    run-code.bat     Claude Code (CLI)    =  run.bat -Client Code
    run-cursor.bat   Cursor               =  run.bat -Client Cursor

Just start/verify the daemon without launching a client:
    run.bat -Client None

STOP THE DAEMON: it keeps running in the background after you close the client
(so it keeps tracking). To stop it cleanly, double-click run-stop.bat (or run
run.bat -Stop) instead of ending it in Task Manager.

NOTE: "Claude Code" (the CLI installed by claude.ai/install.ps1) and
"Claude Desktop" (the GUI app) are different products.


GUI CLIENTS (Claude Desktop / Cursor): FIRST-RUN TRUST STEP
-----------------------------------------------------------
Claude Desktop from claude.ai/download is now a packaged (MSIX) app, and it -
like Cursor - validates TLS against the WINDOWS certificate store. So the first
time you launch a GUI client, the launcher will:
  * import Lumen's local CA into your CurrentUser\Root store (no admin needed),
    so the app trusts Lumen's interception, and
  * for the packaged Claude Desktop, persist HTTPS_PROXY / NODE_EXTRA_CA_CERTS at
    your USER scope so the packaged app inherits them.

This is a man-in-the-middle root CA for your user account - the launcher prints a
security note when it does it. To skip the CA import:  run.bat -NoTrustCA
To undo everything (remove the CA + clear the persisted proxy vars):
    run.bat -Cleanup

Verified working: the Claude Desktop "Code" tab tracks in the dashboard. The
"chat"/"cowork" tabs use a different backend and are not tracked yet.


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

To also undo the GUI first-run trust step (remove Lumen's CA from your Windows
trust store and clear the persisted HTTPS_PROXY / NODE_EXTRA_CA_CERTS vars):
    run.bat -Cleanup
