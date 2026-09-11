Rhizolog {version}, the desktop app: a preview

The Rhizolog server in a window of its own, for Windows. It works. It is a
preview because four things that should be true of a download are not yet:

  - The icons are placeholders.
  - It is not signed, so the first time you run it Windows SmartScreen says
    it does not recognise it. "More info", then "Run anyway", runs it.
  - It needs Microsoft's WebView2 runtime. Windows 11 has it, and most
    Windows 10 machines have it from Windows Update. If yours does not, the
    window opens blank rather than saying what is missing; the Evergreen
    runtime is a free download from Microsoft.
  - The window keeps its browser data in %LOCALAPPDATA%\dev.rhizolog.app,
    not beside the program, so deleting the program leaves that folder
    behind.


Running it

  rhizolog-desktop.exe runs from wherever you put it; nothing is installed.
  The first time, it asks which folder of markdown files to open, and it
  remembers the answer in rhizolog.settings.json beside itself.
  File > Open Wiki... changes it. File > Settings... picks the port and opens
  the log folder, which is the first thing worth attaching to a bug report.


What is in here

  rhizolog-desktop.exe      the app
  LICENSE                   the GNU Affero General Public License, version 3
  THIRD-PARTY-NOTICES.txt   the licences of everything it is built from
  README.txt                this file


An additional permission

  The app is linked with Microsoft's WebView2 loader, which has no source.
  Rhizolog's licence carries this permission for it:

    Additional permission under GNU AGPL version 3 section 7

    If you modify Rhizolog, or any covered work, by linking or combining it
    with the Microsoft Edge WebView2 loader (WebView2Loader.dll or
    WebView2LoaderStatic.lib, as distributed in Microsoft's WebView2 SDK, or
    a modified version of either), containing parts covered by the terms of
    Microsoft's licence for that SDK, the licensors of Rhizolog grant you
    additional permission to convey the resulting work. Corresponding Source
    for a non-source form of such a combination need not include the source
    code of the loader.


The manual, the source and the place to report a bug are all at
https://github.com/grindshell/rhizolog. This is the tag v{version}.
