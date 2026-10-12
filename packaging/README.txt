BoostDeck {VERSION}
====================

Boost every app's sound past 100% on Windows, with an always-on limiter so
nothing clips.

SAFETY
------
Boosted audio with heavy limiting can damage small laptop speakers and your
hearing if played loud for long periods. Start low, especially on
headphones. The red "0 dB" button turns the boost off at once.

BEFORE THE FIRST RUN: INSTALL VB-CABLE
--------------------------------------
BoostDeck sends Windows audio through the free VB-Cable virtual cable. It is
not included; install it yourself:

1. Download VB-CABLE from https://vb-audio.com/Cable/ and run the 64-bit
   setup as administrator, then reboot.
2. Open the classic Sound control panel (Win+R, mmsys.cpl). Note your
   speakers' format under Properties > Advanced (usually 48000 Hz).
3. Set CABLE Input (Playback tab) and CABLE Output (Recording tab) to that
   same sample rate.
4. On CABLE Output > Listen, make sure "Listen to this device" is off, or
   you will hear everything twice.
5. On CABLE Input > Advanced, untick "Allow applications to take exclusive
   control of this device".

If VB-Cable is missing, BoostDeck shows these steps on its setup screen.

RUNNING IT
----------
Run boostdeck.exe and press "Boost off" to turn boost on. BoostDeck makes
CABLE Input the Windows default output while boost is on, and puts your own
device back when you turn it off or close the window. If BoostDeck is ever
killed, the next launch restores your device; to do it by hand, set your
speakers as the default output in Sound settings.

BoostDeck is not signed yet. Windows may say "Windows protected your PC":
choose "More info", then "Run anyway". On PCs with Smart App Control turned
on, the installer is blocked; use this portable version instead.

FILES
-----
Settings and presets: %APPDATA%\BoostDeck\BoostDeck\config
Logs (last 5 runs):   %LOCALAPPDATA%\BoostDeck\logs

KNOWN LIMITATIONS
-----------------
- About 35-80 ms of added delay: fine for music and video, noticeable in
  fast games.
- Apps using WASAPI exclusive mode or ASIO bypass the cable and are not
  boosted.
- While boost is on, the Windows volume slider controls CABLE Input, not
  your speakers.
- Stereo only.

License: MIT (see LICENSE.txt).
