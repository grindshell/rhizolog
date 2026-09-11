Rhizolog {version}, the server

A wiki over a directory of markdown files, with an HTTP API that is meant to
be used by you, and by whatever agents you point at it. This is the server,
with the dashboard compiled into it: one program, and nothing to install.


Running it

  Point RHIZOLOG_ROOT at a folder of markdown files and start it.

    Windows, in PowerShell:
      $env:RHIZOLOG_ROOT = "C:\Users\you\notes"
      .\rhizolog.exe

    Linux:
      RHIZOLOG_ROOT=~/notes ./rhizolog

  It prints the address it is listening on, normally http://127.0.0.1:3000,
  and the dashboard is there in any browser. Ctrl+C stops it. Without
  RHIZOLOG_ROOT the wiki is a folder called "wiki" in whichever directory you
  started it from.

  A wiki with no accounts is open: nobody signs in and nothing is refused.
  That is safe because it listens on 127.0.0.1, which only this computer can
  reach. Read "Keeping it private" in the README before changing that.

  On Windows, the program is not signed, so Windows may say it does not
  recognise it.


What is in here

  rhizolog (.exe on Windows)  the server
  LICENSE                     the GNU Affero General Public License, version 3
  THIRD-PARTY-NOTICES.txt     the licences of everything it is built from
  README.txt                  this file


The manual, the source and the place to report a bug are all at
https://github.com/grindshell/rhizolog. This is the tag v{version}.
