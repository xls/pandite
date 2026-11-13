# Text in images

pandite reads the text in the image on screen, so you can select it and
copy it like text in a document. A photo of a sign, a scanned page or a
screenshot becomes text you can paste into a note or a search box.

Recognition runs on your computer with the text recognizer that is part
of the operating system. Nothing is sent anywhere, and pandite works the
same without a network connection.

pandite reads the text after the image is on screen at full resolution
and stays there for a quarter of a second. Recognition runs in the
background at a low priority, so the image always comes first. When you
move to another image, pandite stops the recognition. pandite keeps the
text of recently viewed images until it closes, so an image that you come
back to has its text at once.

The recognizer reads upright text best. Sideways or upside-down text gives
little or no text. When you rotate or flip the image, pandite reads the
text again the way the image now stands. The new text replaces the old
text. So a photo that opens on its side gets its text when you turn it
upright. The same rules apply: the turned image comes first, stays on
screen for a quarter of a second, and then pandite reads it in the
background. A crop or a resize does not start a new reading. pandite also
keeps the text of each way you turned the image, so when you turn the
image back, or undo a turn, the earlier text returns at once.

## Select and copy text

1. Open an image that shows text.
2. Move the pointer over a word. When pandite has read the text, the
   pointer becomes a text cursor.
3. Select the text. Use one of these methods:
   - Drag from the first word to the last word. The selection follows the
     reading order, from line to line.
   - Double-click a word to select the word.
   - Triple-click a word to select its line.
   - Double-click a word and keep the button down after the second click.
     Drag to add whole words, from that word to the word at the pointer.
     Release the button to stop.
   - Triple-click a word and keep the button down after the third click.
     Drag to add whole lines, from that line to the line at the pointer.
     Release the button to stop.
   - Press Ctrl+A to select all the text in the image.
4. Press Ctrl+C to copy the selected text. Or right-click and select Copy
   selected text.

The copied text has one line for each line in the image, and one space
between the words of a line.

A drag that starts on the image away from the text pans the image, as
before. Click the image away from the text to clear the selection. Esc
closes pandite, also while text is selected.

The selection stays on the words when you zoom, pan or crop the image.
Words that a crop removes are not selected and not copied. When you rotate
or flip the image, the selection stays on the words until the text of the
turned image replaces them. Then the selection clears.

## Shortcuts

| Keys or action | Result |
|---|---|
| Drag from a word | Select words from that word to the pointer |
| Double-click a word | Select the word |
| Triple-click a word | Select the line |
| Double-click a word, keep the button down and drag | Select whole words from that word to the pointer |
| Triple-click a word, keep the button down and drag | Select whole lines from that line to the pointer |
| Ctrl+A | Select all the text in the image |
| Ctrl+C | Copy the selected text. Without a selection: copy the image |
| Ctrl+Shift+C | Copy the file path, also while text is selected |
| Click the image away from the text | Clear the selection |
| Esc | Close pandite (a menu or dialog closes first) |

## Platforms and limits

- Windows 10 and 11 only, for now. On macOS and Linux, pandite shows no
  text features.
- Windows reads text in the OCR languages that are installed. Windows
  installs one with most display languages.
- Without an OCR language, pandite shows no text features and no error.
- pandite reads still images. It does not read animations.
- Text that is not upright gives little or no text. Rotate or flip the
  image until the text is upright. pandite then reads it again.
- pandite reads large images at 6000 pixels on the long side. Very small
  print in a very large photo can be lost. Zooming in does not change
  this.

## Add an OCR language on Windows

1. Open Settings. Select Time and language, then Language and region.
2. Select Add a language, then select the language.
3. In the optional features, make sure that Optical character recognition
   is selected.
4. Select Install.
5. Restart pandite.

pandite reads the languages of your user profile that have OCR data.

## Turn text recognition off

1. Close pandite.
2. Open `settings.ini` in a text editor. The README gives its folder for
   each system.
3. Find the line `ocr = auto` and change it to `ocr = off`. If the line is
   missing, add `ocr = off` at the end.
4. Save the file and start pandite.

With `ocr = off`, pandite reads no text and shows no text features.
