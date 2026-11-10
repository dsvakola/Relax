; Relax - Your data is safe! : installer hooks
; Developed by Prof. Dattaraj S. Vidyasagar & Prof. Yash D. Vidyasagar, for Vidyasagar Academy.
;
; Sharp installer pictures on every screen scaling (100% ... 300%).
; By default Modern UI stretches a small picture with a "nearest pixel" method, which looks blurry and
; jagged on high-DPI screens. Instead, the installer carries large (3x) pictures and, when it starts,
; scales them ONCE to the exact pixel size of the picture areas on this screen using Windows' high-quality
; HALFTONE filter. Modern UI then shows them 1:1, without any further stretching.

!define MUI_WELCOMEFINISHPAGE_BITMAP_STRETCH NoStretchNoCropNoAlign

; Clear, short texts on the installer pages.
!define MUI_WELCOMEPAGE_TITLE "Welcome to Relax"
!define MUI_WELCOMEPAGE_TEXT "This will install Relax - Your data is safe! on your computer.$\r$\n$\r$\nRelax is a 100% offline backup and folder synchronization tool. Your files never leave your PC and your drives.$\r$\n$\r$\nDeveloped by Prof. Dattaraj S. Vidyasagar and Prof. Yash D. Vidyasagar, for Vidyasagar Academy.$\r$\n$\r$\nClick Next to continue."
!define MUI_LICENSEPAGE_TEXT_TOP "Please read the licence terms of Relax."
!define MUI_LICENSEPAGE_TEXT_BOTTOM "If you accept the terms, click I Agree to install Relax."
!define MUI_DIRECTORYPAGE_TEXT_TOP "Relax will be installed in the folder below. To choose a different folder, click Browse."
!define MUI_FINISHPAGE_TITLE "Relax is installed"
!define MUI_FINISHPAGE_TEXT "Relax - Your data is safe! is ready to use.$\r$\n$\r$\nClick Finish to close Setup."
!define MUI_CUSTOMFUNCTION_GUIINIT Relax.GuiInit
!define MUI_CUSTOMFUNCTION_UNGUIINIT un.Relax.GuiInit

; Scales the bitmap file $R0 to $R2 x $R3 pixels (HALFTONE) and writes it as a 24-bit BMP to $R1.
!macro RELAX_SCALE_BMP_FUNC UN
Function ${UN}Relax.ScaleBmp
  IntCmp $R2 0 done done
  IntCmp $R3 0 done done
  ; Source picture (LR_LOADFROMFILE | LR_CREATEDIBSECTION)
  System::Call 'user32::LoadImageW(p 0, w R0, i 0, i 0, i 0, i 0x2010) p .R4'
  StrCmp $R4 0 done
  System::Call '*(i 0, i 0, i 0, i 0, i 0, i 0, i 0, i 0) p .r9'
  System::Call 'gdi32::GetObjectW(p R4, i 32, p r9)'
  System::Call '*$9(i, i .R5, i .R6)'
  System::Free $9
  ; Device contexts
  System::Call 'user32::GetDC(p 0) p .R7'
  System::Call 'gdi32::CreateCompatibleDC(p R7) p .R8'
  System::Call 'gdi32::CreateCompatibleDC(p R7) p .R9'
  System::Call 'gdi32::SelectObject(p R8, p R4) p .r7'
  ; Destination: 24-bit bottom-up DIB of the exact size
  IntOp $3 $R2 * 3
  IntOp $3 $3 + 3
  IntOp $3 $3 & 0xFFFFFFFC
  IntOp $4 $3 * $R3
  System::Call '*(i 40, i R2, i R3, &i2 1, &i2 24, i 0, i r4, i 3780, i 3780, i 0, i 0) p .r2'
  System::Call 'gdi32::CreateDIBSection(p R7, p r2, i 0, *p .r1, p 0, i 0) p .r0'
  System::Call 'gdi32::SelectObject(p R9, p r0) p .r8'
  ; High-quality downscale
  System::Call 'gdi32::SetStretchBltMode(p R9, i 4)'
  System::Call 'gdi32::SetBrushOrgEx(p R9, i 0, i 0, p 0)'
  System::Call 'gdi32::StretchBlt(p R9, i 0, i 0, i R2, i R3, p R8, i 0, i 0, i R5, i R6, i 0x00CC0020)'
  System::Call 'gdi32::GdiFlush()'
  ; Write the BMP file: 14-byte file header + 40-byte info header + pixels
  IntOp $5 $4 + 54
  FileOpen $9 $R1 w
  ; "BM", file size (4 bytes, little endian), 4 reserved bytes, pixel offset 54 (4 bytes)
  FileWriteByte $9 66
  FileWriteByte $9 77
  IntOp $6 $5 & 0xFF
  FileWriteByte $9 $6
  IntOp $6 $5 >> 8
  IntOp $6 $6 & 0xFF
  FileWriteByte $9 $6
  IntOp $6 $5 >> 16
  IntOp $6 $6 & 0xFF
  FileWriteByte $9 $6
  IntOp $6 $5 >> 24
  IntOp $6 $6 & 0xFF
  FileWriteByte $9 $6
  FileWriteByte $9 0
  FileWriteByte $9 0
  FileWriteByte $9 0
  FileWriteByte $9 0
  FileWriteByte $9 54
  FileWriteByte $9 0
  FileWriteByte $9 0
  FileWriteByte $9 0
  System::Call 'kernel32::WriteFile(p r9, p r2, i 40, *i 0, p 0)'
  System::Call 'kernel32::WriteFile(p r9, p r1, i r4, *i 0, p 0)'
  FileClose $9
  ; Clean up
  System::Call 'gdi32::SelectObject(p R9, p r8)'
  System::Call 'gdi32::SelectObject(p R8, p r7)'
  System::Call 'gdi32::DeleteObject(p r0)'
  System::Call 'gdi32::DeleteObject(p R4)'
  System::Call 'gdi32::DeleteDC(p R8)'
  System::Call 'gdi32::DeleteDC(p R9)'
  System::Call 'user32::ReleaseDC(p 0, p R7)'
  System::Free $2
  done:
FunctionEnd
!macroend

!macro RELAX_PUSH_ALL
  Push $0
  Push $1
  Push $2
  Push $3
  Push $4
  Push $5
  Push $6
  Push $7
  Push $8
  Push $9
  Push $R0
  Push $R1
  Push $R2
  Push $R3
  Push $R4
  Push $R5
  Push $R6
  Push $R7
  Push $R8
  Push $R9
!macroend

!macro RELAX_POP_ALL
  Pop $R9
  Pop $R8
  Pop $R7
  Pop $R6
  Pop $R5
  Pop $R4
  Pop $R3
  Pop $R2
  Pop $R1
  Pop $R0
  Pop $9
  Pop $8
  Pop $7
  Pop $6
  Pop $5
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Pop $0
!macroend

; Header picture (top right of the inner pages): scaled to the exact size of control 1046.
!macro RELAX_SHARP_HEADER UN
  InitPluginsDir
  File "/oname=$PLUGINSDIR\relax-header-hi.bmp" "${__FILEDIR__}\header-hi.bmp"
  GetDlgItem $0 $HWNDPARENT 1046
  System::Call '*(i 0, i 0, i 0, i 0) p .r1'
  System::Call 'user32::GetClientRect(p r0, p r1)'
  System::Call '*$1(i, i, i .R2, i .R3)'
  System::Free $1
  StrCpy $R0 "$PLUGINSDIR\relax-header-hi.bmp"
  StrCpy $R1 "$PLUGINSDIR\relax-header.bmp"
  Call ${UN}Relax.ScaleBmp
  IfFileExists "$PLUGINSDIR\relax-header.bmp" 0 +2
    SetBrandingImage /IMGID=1046 "$PLUGINSDIR\relax-header.bmp"
!macroend

!insertmacro RELAX_SCALE_BMP_FUNC ""
!insertmacro RELAX_SCALE_BMP_FUNC "un."

Function Relax.GuiInit
  !insertmacro RELAX_PUSH_ALL
  ; Welcome / Finish side picture: 109 x 193 dialog units, converted to pixels for this screen.
  InitPluginsDir
  File "/oname=$PLUGINSDIR\relax-sidebar-hi.bmp" "${__FILEDIR__}\sidebar-hi.bmp"
  System::Call '*(i 0, i 0, i 109, i 193) p .r1'
  System::Call 'user32::MapDialogRect(p $HWNDPARENT, p r1)'
  System::Call '*$1(i, i, i .R2, i .R3)'
  System::Free $1
  StrCpy $R0 "$PLUGINSDIR\relax-sidebar-hi.bmp"
  StrCpy $R1 "$PLUGINSDIR\relax-sidebar.bmp"
  Call Relax.ScaleBmp
  ; Modern UI loads $PLUGINSDIR\modern-wizard.bmp when the Welcome and Finish pages open.
  IfFileExists "$PLUGINSDIR\relax-sidebar.bmp" 0 +2
    CopyFiles /SILENT "$PLUGINSDIR\relax-sidebar.bmp" "$PLUGINSDIR\modern-wizard.bmp"
  !insertmacro RELAX_SHARP_HEADER ""
  !insertmacro RELAX_POP_ALL
FunctionEnd

Function un.Relax.GuiInit
  !insertmacro RELAX_PUSH_ALL
  !insertmacro RELAX_SHARP_HEADER "un."
  !insertmacro RELAX_POP_ALL
FunctionEnd
