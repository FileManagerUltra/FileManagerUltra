; Notify Explorer after an interactive install closes, without restarting it or
; deleting user-managed shortcuts, pins or the global icon cache.
Function .onGUIEnd
  IfFileExists "$INSTDIR\${MAINBINARYNAME}.exe" 0 done
    System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
  done:
FunctionEnd
