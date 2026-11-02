package com.mansfera.mailbub

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat

/**
 * Keeps the app running while a sign-in is open in the browser. In the background Android
 * freezes the app and cuts its network, but the browser returns to a local port the app
 * listens on, and the app then has to fetch its tokens. Android ends the service after three
 * minutes at most; without the notification permission it simply shows nothing.
 */
class SignInService : Service() {
  override fun onBind(intent: Intent?): IBinder? = null

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    val channel = "sign-in"
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
      getSystemService(NotificationManager::class.java).createNotificationChannel(
        NotificationChannel(channel, getString(R.string.signing_in), NotificationManager.IMPORTANCE_LOW)
      )
    }
    val notification = NotificationCompat.Builder(this, channel)
      .setSmallIcon(android.R.drawable.ic_dialog_email)
      .setContentTitle(getString(R.string.signing_in))
      .setOngoing(true)
      .build()
    val type =
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) ServiceInfo.FOREGROUND_SERVICE_TYPE_SHORT_SERVICE
      else 0
    ServiceCompat.startForeground(this, 1, notification, type)
    return START_NOT_STICKY
  }

  @Deprecated("Android 15 adds a variant; this one is still what runs for a short service")
  override fun onTimeout(startId: Int) {
    stopSelf()
  }
}
