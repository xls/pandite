# Keyboard and mouse

The keyboard table below is the shortcut table of the interface
(`SHORTCUTS` in `crates/pandite-ui/src/command.rs`), with the command and key
names that pandite shows. When a command has more than one key, the first
key is the key that tooltips and menus show. Press F1 in pandite to see the
same list.

On macOS, Ctrl is the Command key and Alt is the Option key.

## Keyboard

| Command | Keys |
|---|---|
| Open... | Ctrl+O |
| Save as... | Ctrl+Shift+S, Ctrl+S |
| Copy file path | Ctrl+C |
| Copy selected text | Ctrl+C |
| Select all text | Ctrl+A |
| Undo | Ctrl+Z |
| Rotate left | Shift+R, L |
| Rotate right | R |
| Flip horizontal | H |
| Flip vertical | V |
| Crop | C |
| Resize... | Ctrl+R |
| Bulk convert and resize... | Ctrl+B |
| Fit to window | F, 0 |
| Actual size (1:1) | 1 |
| Zoom in | +, = |
| Zoom out | - |
| Pan up | Up, Ctrl+Up |
| Pan down | Down, Ctrl+Down |
| Pan left | Ctrl+Left |
| Pan right | Ctrl+Right |
| Previous image | Left, PgUp, Backspace |
| Next image | Right, PgDn, Space |
| First image | Home |
| Last image | End |
| Gallery strip | G |
| Full screen | F11, Alt+Enter |
| Context menu | Menu, Shift+F10 |
| Keyboard shortcuts | F1 |
| Cancel or leave full screen | Esc |
| Quit | Ctrl+Q, Ctrl+W |

### In crop mode

| Command | Keys |
|---|---|
| Apply crop | Enter |
| Select whole image | Ctrl+A |
| Cancel crop | Esc |
| Move the selection 1 pixel | Left, Right, Up, Down |
| Move the selection 10 pixels | Shift+Left, Shift+Right, Shift+Up, Shift+Down |

### Rules

- The keys +, -, = and the digits ignore Shift, so keyboard layouts that
  need Shift for these keys work.
- When you hold the key, these commands repeat: previous image, next image,
  zoom in, zoom out and the four pan commands. Other commands run one time.
- Ctrl+C copies the selected text when text in the image is selected.
  Otherwise, Ctrl+C copies the file path.
- Ctrl+A and Copy selected text work only on Windows, when pandite has read
  the text of the image. See [text-recognition.md](text-recognition.md).
  On other systems, the shortcut overlay does not show them.
- When a text selection exists and no menu or dialog is open, Esc clears
  the selection first. The next Esc has its usual meaning.
- When a file is loading, Esc stops the load. If the file is the first image,
  pandite closes. If the file is a later image, pandite shows the previous image.
- Esc closes the topmost menu, dialog or mode first. When nothing is open,
  Esc leaves full screen.
- In a dialog, Full screen and Quit stay available. The dialog receives all
  other keys.
- In crop mode, these commands stay available: fit, 1:1, zoom, pan, full
  screen, gallery strip, context menu, keyboard shortcuts, cancel, quit and
  the crop commands.
- The arrow keys move the selection only when a selection exists.

## Mouse

| Action | Result |
|---|---|
| Wheel over the image | Zoom at the pointer, 1.2 times for each wheel notch |
| Wheel over the gallery strip | Scroll the strip |
| Drag the image with the left or middle button | Pan |
| Drag from a word with the left button | Select the words from that word to the word nearest the pointer, in reading order |
| Double-click a word | Select the word |
| Triple-click a word | Select the line of the word |
| Click the image away from the text | Clear the text selection |
| Double-click the image away from the text | In fit to window: 1:1 at the pointer. In other modes: fit to window. |
| Right-click | Context menu |
| Back and Forward side buttons | Previous image and next image |
| Click a thumbnail | Show that image |
| Drag the gallery strip | Scroll the strip |

The text actions work only where pandite read text in the image (on
Windows). Over a word, the pointer is a text cursor. The middle button
pans also over text.

### In crop mode

| Action | Result |
|---|---|
| Drag on the image | Draw a new selection |
| Drag inside the selection | Move the selection |
| Drag a handle or an edge | Resize the selection |
| Hold Shift while you drag | Keep a new selection or a corner drag square |
