package com.mansfera.mailbub

import android.app.Activity
import android.content.Intent
import android.graphics.drawable.ColorDrawable
import android.net.Uri
import android.webkit.MimeTypeMap
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import androidx.core.view.WindowCompat
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin
import java.io.File

@InvokeArg
class OpenArgs {
  lateinit var target: String
}

@InvokeArg
class BarsArgs {
  var dark: Boolean = false
}

@InvokeArg
class SignInArgs {
  var active: Boolean = false
}

/** What the Rust core needs from Android itself. */
@TauriPlugin
class SystemPlugin(private val activity: Activity) : Plugin(activity) {
  /** Hands a link or a cached file to the app the system picks (`files::open_with_system`). */
  @Command
  fun open(invoke: Invoke) {
    try {
      val target = invoke.parseArgs(OpenArgs::class.java).target
      val intent =
        if (target.startsWith("/")) fileIntent(File(target))
        else Intent(Intent.ACTION_VIEW, Uri.parse(target))
      activity.startActivity(intent)
      invoke.resolve()
    } catch (e: Exception) {
      invoke.reject(e.message)
    }
  }

  /** Other apps can't read file:// paths; FileProvider grants them this one file. */
  private fun fileIntent(file: File): Intent {
    val uri = FileProvider.getUriForFile(activity, "${activity.packageName}.fileprovider", file)
    val mime = MimeTypeMap.getSingleton().getMimeTypeFromExtension(file.extension.lowercase())
    return Intent(Intent.ACTION_VIEW)
      .setDataAndType(uri, mime ?: "*/*")
      .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
  }

  /** Runs `SignInService` for as long as a sign-in is open in the browser. */
  @Command
  fun setSignInActive(invoke: Invoke) {
    try {
      val service = Intent(activity, SignInService::class.java)
      if (invoke.parseArgs(SignInArgs::class.java).active) {
        ContextCompat.startForegroundService(activity, service)
      } else {
        activity.stopService(service)
      }
      invoke.resolve()
    } catch (e: Exception) {
      invoke.reject(e.message)
    }
  }

  /** The strips behind the system bars follow the app's theme, not only the system's. */
  @Command
  fun setBars(invoke: Invoke) {
    val dark = invoke.parseArgs(BarsArgs::class.java).dark
    activity.runOnUiThread {
      val window = activity.window
      // The sidebar colours in index.css.
      window.setBackgroundDrawable(ColorDrawable(if (dark) 0xFF352D2F.toInt() else 0xFFF4EBD9.toInt()))
      WindowCompat.getInsetsController(window, window.decorView).apply {
        isAppearanceLightStatusBars = !dark
        isAppearanceLightNavigationBars = !dark
      }
      invoke.resolve()
    }
  }
}
