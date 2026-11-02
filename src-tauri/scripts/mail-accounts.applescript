-- Prints one line per Apple Mail account, fields separated by ASCII 31.
-- Never reads passwords.
set sep to character id 31
set rows to {}
tell application "Mail"
	repeat with a in every account
		set f to {}
		try
			set end of f to (class of a) as text
		on error
			set end of f to ""
		end try
		try
			set end of f to (name of a) as text
		on error
			set end of f to ""
		end try
		try
			set end of f to (enabled of a) as text
		on error
			set end of f to ""
		end try
		try
			set AppleScript's text item delimiters to ","
			set end of f to (email addresses of a) as text
			set AppleScript's text item delimiters to ""
		on error
			set end of f to ""
		end try
		try
			set end of f to (full name of a) as text
		on error
			set end of f to ""
		end try
		try
			set end of f to (user name of a) as text
		on error
			set end of f to ""
		end try
		try
			set end of f to (server name of a) as text
		on error
			set end of f to ""
		end try
		try
			set end of f to (port of a) as text
		on error
			set end of f to ""
		end try
		try
			set end of f to (uses ssl of a) as text
		on error
			set end of f to ""
		end try
		set s to missing value
		try
			set s to smtp server of a
		end try
		if s is missing value then
			set f to f & {"", "", "", ""}
		else
			try
				set end of f to (server name of s) as text
			on error
				set end of f to ""
			end try
			try
				set end of f to (port of s) as text
			on error
				set end of f to ""
			end try
			try
				set end of f to (uses ssl of s) as text
			on error
				set end of f to ""
			end try
			try
				set end of f to (user name of s) as text
			on error
				set end of f to ""
			end try
		end if
		set AppleScript's text item delimiters to sep
		set end of rows to f as text
		set AppleScript's text item delimiters to ""
	end repeat
end tell
set AppleScript's text item delimiters to linefeed
return rows as text
